#!/usr/bin/env python3

import platform
import subprocess
import shutil
import os
import sys

MMTk_ROOT = os.path.join(__file__, "..", "..")

PLANS = []

# Find all plans from options.rs
options = ""
with open(os.path.abspath(os.path.join(MMTk_ROOT, "src", "util", "options.rs")), 'r') as file:
    options = file.read().replace('\n', '')
import re
search = re.search("enum PlanSelector \{([^\}]*)\}", options)
if search:
    raw_plans = search.group(1)
    # Python split() results in an empty string as the last element. Use filter() to remove it.
    PLANS = list(filter(None, [x.strip() for x in raw_plans.split(",")]))
else:
    print("cannot find PlanSelector in options.rs")
    sys.exit(1)

os.chdir(os.path.abspath(MMTk_ROOT))

extra_features = ""
if len(sys.argv) > 1:
    extra_features = sys.argv[1]


def exec_and_redirect(args, env=None):
    print("[exec_and_redirect] {} (env = {})".format(args, env))
    p = subprocess.Popen(args,
                         env=env)
    p.wait()
    if p.returncode != 0:
        exit(p.returncode)

# Get the active toolchain, something like this: stable-x86_64-unknown-linux-gnu
active_toolchain = str(subprocess.check_output(["rustup", "show", "active-toolchain"]).decode('utf-8')).split(' ')[0]
print("Active rust toolchain: " + active_toolchain)
if "x86_64" in active_toolchain:
    m32 = False
elif "i686" in active_toolchain:
    m32 = True
else:
    print("Unknown toolchain: " + active_toolchain)
    sys.exit(1)

system = platform.system()
assert system == "Darwin" or system == "Linux"

SUFFIX = ".so"
if system == "Darwin":
    SUFFIX = ".dylib"
elif system == "Linux":
    SUFFIX = ".so"

LIBRARY_PATH = "LD_LIBRARY_PATH"
if system == "Darwin":
    LIBRARY_PATH = "DYLD_LIBRARY_PATH"
elif system == "Linux":
    LIBRARY_PATH = "LD_LIBRARY_PATH"

vmbinding = "vmbindings/dummyvm"

cmd = []
cmd.append("cargo")
cmd.extend([
    "build",
    "--manifest-path",
    "vmbindings/dummyvm/Cargo.toml",
    "--features", " ".join(extra_features)
])

exec_and_redirect(cmd)
exec_and_redirect(cmd + ["--release"])
shutil.copyfile("{}/target/release/libmmtk_dummyvm{}".format(vmbinding, SUFFIX),
                "./libmmtk{}".format(SUFFIX))

cmd = [
    "gcc",
    "./examples/main.c",
    "-lmmtk",
    "-L.",
    "-I{}/api".format(vmbinding),
    "-O3",
    "-o",
    "test_mmtk",
]
if m32:
    cmd.append("-m32")

exec_and_redirect(cmd)

for plan in PLANS:
    exec_and_redirect(["./test_mmtk"], env={LIBRARY_PATH: ".", "MMTK_PLAN": plan})

os.remove("./test_mmtk")
