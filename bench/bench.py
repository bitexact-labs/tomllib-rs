"""Speed: stdlib tomllib vs tomllib_rs on realistic documents (built here, deterministic)."""
import random, time, tomllib
import tomllib_rs
random.seed(5)

def lockfile(n):   # Cargo.lock / poetry.lock style: many [[package]] tables
    out = ["version = 3", ""]
    for i in range(n):
        out += ["[[package]]", f'name = "pkg-{i}"', f'version = "{random.randint(0,9)}.{random.randint(0,40)}.{random.randint(0,20)}"',
                'source = "registry+https://github.com/rust-lang/crates.io-index"',
                f'checksum = "{random.getrandbits(256):064x}"',
                "dependencies = [" + ", ".join(f'"dep-{random.randint(0, n)}"' for _ in range(random.randint(0, 6))) + "]", ""]
    return "\n".join(out)

def config(n):   # wide config: nested tables, inline tables, numbers, dates, multiline strings
    out = []
    for i in range(n):
        out += [f"[service.s{i}]", f"port = {random.randint(1000, 65000)}", f"ratio = {random.random():.6f}", "enabled = true",
                f"started = 2026-11-0{random.randint(1,9)}T{random.randint(0,23):02d}:15:00Z",
                f'limits = {{ cpu = {random.randint(1,16)}, mem = "{random.randint(1,64)}G", burst = [1, 2, 3] }}',
                'description = """', "A multi-line", "description\\", "  continued", '"""', f"tags = [\"a{i}\", \"b\", 'c']", ""]
    return "\n".join(out)

docs = [("lockfile, 3000 packages", lockfile(3000)), ("config, 2000 tables", config(2000)), ("small pyproject-like", config(5))]
print("| Document | Size | stdlib tomllib | tomllib_rs | Speedup | Same result |\n|---|---|---|---|---|---|")
for name, d in docs:
    reps = 200 if len(d) < 5000 else 5
    t = time.perf_counter()
    for _ in range(reps): a = tomllib.loads(d)
    t1 = (time.perf_counter() - t) / reps
    t = time.perf_counter()
    for _ in range(reps): b = tomllib_rs.loads(d)
    t2 = (time.perf_counter() - t) / reps
    print(f"| {name} | {len(d)/1024:.0f} KB | {t1*1000:.1f} ms | {t2*1000:.2f} ms | {t1/t2:.1f}× | {'yes' if repr(a) == repr(b) else 'NO'} |")
