"""Compare tomllib_rs with the stdlib tomllib.
  compare.py suite DIR          every *.toml file under DIR (toml-test valid + invalid)
  compare.py fuzz N SEED DIR    N random mutations of the suite files
Equal means: same Python value (compared by repr, so nan and types count), or the same exception type, message and pos."""
import os, random, sys, tomllib
import tomllib_rs

def run(mod, data):
    try:
        return ("ok", repr(mod.loads(data)))
    except Exception as e:
        return ("err", type(e).__name__, str(e), getattr(e, "pos", None), getattr(e, "lineno", None), getattr(e, "colno", None))

def files(d):
    out = []
    for root, _, fs in os.walk(d):
        for f in fs:
            if f.endswith(".toml"):
                b = open(os.path.join(root, f), "rb").read()
                try:
                    out.append((os.path.join(root, f), b.decode()))
                except UnicodeDecodeError:
                    pass
    return sorted(out)

mode = sys.argv[1]
bad = total = 0
if mode == "suite":
    for path, text in files(sys.argv[2]):
        total += 1
        a, b = run(tomllib, text), run(tomllib_rs, text)
        if a != b:
            bad += 1; print("MISMATCH", path, "\n  std:", a[:3], "\n  rs: ", b[:3])
else:
    n, seed, d = int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
    random.seed(seed); corpus = [t for _, t in files(d)]
    ALPH = list("[]{}=.,\"'#\n \t\\_-+:0123456789abcdefxtTZzunie") + ["\x00", "\x7f", "\r\n", "é", " ", "😀", "\"\"\"", "'''"]
    for i in range(n):
        s = list(random.choice(corpus))
        for _ in range(random.randint(1, 4)):
            k = random.random(); p = random.randrange(len(s) + 1)
            if k < .35: s[p:p] = [random.choice(ALPH)]
            elif k < .7 and s: del s[min(p, len(s) - 1)]
            elif s: s[min(p, len(s) - 1)] = random.choice(ALPH)
        text = "".join(s); total += 1
        a, b = run(tomllib, text), run(tomllib_rs, text)
        if a != b:
            bad += 1; print("MISMATCH", i, repr(text[:200]), "\n  std:", a[:4], "\n  rs: ", b[:4], flush=True)
            if bad >= 8: break
print(f"{mode}: cases={total} mismatches={bad}")
sys.exit(1 if bad else 0)
