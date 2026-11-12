use pyo3::exceptions::PyIndexError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyString};
use std::collections::HashMap;
use tomllib_core::{loads, Error, Table, Tz, Value};

struct Ctx<'py> {
    py: Python<'py>,
    dt: Bound<'py, PyModule>,
    tz_cache: HashMap<(i32, u32, u32), Py<PyAny>>,
}

fn int_obj<'py>(py: Python<'py>, text: &str) -> PyResult<Bound<'py, PyAny>> {
    let clean: String = text.chars().filter(|&c| c != '_').collect();
    let (neg, body) = match clean.strip_prefix('-') {
        Some(b) => (true, b.to_string()),
        None => (false, clean.trim_start_matches('+').to_string()),
    };
    let parsed = if let Some(h) = body.strip_prefix("0x") {
        i64::from_str_radix(h, 16).ok()
    } else if let Some(b) = body.strip_prefix("0b") {
        i64::from_str_radix(b, 2).ok()
    } else if let Some(o) = body.strip_prefix("0o") {
        i64::from_str_radix(o, 8).ok()
    } else {
        body.parse::<i64>().ok()
    };
    match parsed {
        Some(v) => Ok((if neg { -v } else { v }).into_pyobject(py)?.into_any()),
        None => py.import("builtins")?.getattr("int")?.call1((text, 0)),
    }
}

fn float_val(text: &str) -> f64 {
    if text.ends_with("nan") {
        return if text.starts_with('-') { -f64::NAN } else { f64::NAN };
    }
    let clean: String = text.chars().filter(|&c| c != '_').collect();
    clean.parse::<f64>().expect("regex guarantees a valid float")
}

impl<'py> Ctx<'py> {
    fn tz(&mut self, tz: Tz) -> PyResult<Py<PyAny>> {
        Ok(match tz {
            Tz::Local => self.py.None(),
            Tz::Utc => self.dt.getattr("timezone")?.getattr("utc")?.unbind(),
            Tz::Offset { sign, h, m } => {
                if let Some(t) = self.tz_cache.get(&(sign, h, m)) {
                    return Ok(t.clone_ref(self.py));
                }
                let kw = PyDict::new(self.py);
                kw.set_item("hours", sign * h as i32)?;
                kw.set_item("minutes", sign * m as i32)?;
                let td = self.dt.getattr("timedelta")?.call((), Some(&kw))?;
                let t = self.dt.getattr("timezone")?.call1((td,))?.unbind();
                self.tz_cache.insert((sign, h, m), t.clone_ref(self.py));
                t
            }
        })
    }

    fn val(&mut self, v: &Value) -> PyResult<Bound<'py, PyAny>> {
        let py = self.py;
        Ok(match v {
            Value::Str(s) => PyString::new(py, s).into_any(),
            Value::Int(t) => int_obj(py, t)?,
            Value::Float(t) => float_val(t).into_pyobject(py)?.into_any(),
            Value::Bool(b) => b.into_pyobject(py)?.to_owned().into_any(),
            Value::Date { y, m, d } => self.dt.getattr("date")?.call1((*y, *m, *d))?,
            Value::Time { h, mi, s, us } => self.dt.getattr("time")?.call1((*h, *mi, *s, *us))?,
            Value::DateTime { y, m, d, h, mi, s, us, tz } => {
                let tzo = self.tz(*tz)?;
                self.dt.getattr("datetime")?.call1((*y, *m, *d, *h, *mi, *s, *us, tzo))?
            }
            Value::Array(a) => {
                let l = PyList::empty(py);
                for x in a {
                    l.append(self.val(x)?)?;
                }
                l.into_any()
            }
            Value::Table(t) => self.table(t)?.into_any(),
        })
    }

    fn table(&mut self, t: &Table) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(self.py);
        for (k, v) in t {
            d.set_item(k, self.val(v)?)?;
        }
        Ok(d)
    }
}

/// Parse an already normalised document (CRLF replaced by LF). Raises tomllib.TOMLDecodeError like the stdlib.
#[pyfunction]
fn parse<'py>(py: Python<'py>, src: &Bound<'py, PyString>) -> PyResult<Bound<'py, PyDict>> {
    let text = src.to_str()?.to_owned();
    let res = py.detach(|| loads(&text));
    match res {
        Ok(t) => {
            let mut ctx = Ctx { py, dt: py.import("datetime")?, tz_cache: HashMap::new() };
            ctx.table(&t)
        }
        Err(Error::Decode { msg, pos }) => {
            let cls = py.import("tomllib")?.getattr("TOMLDecodeError")?;
            Err(PyErr::from_value(cls.call1((msg, src, pos))?))
        }
        Err(Error::Index) => Err(PyIndexError::new_err("list index out of range")),
    }
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    Ok(())
}
