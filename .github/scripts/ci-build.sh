set -xe

# Execute this script under the root folder of this repo. Otherwise it will fail.

# Build plans
cargo build --features nogc
cargo build --features semispace

# Build features
cargo build --features nogc,vm_space
cargo build --features nogc,vm_space,code_space,ro_space
cargo build --features nogc,nogc_lock_free
cargo build --features nogc,nogc_lock_free,nogc_no_zeroing
cargo build --features semispace,vm_space
cargo build --features semispace,vm_space,code_space,ro_space
cargo build --features nogc,sanity
cargo build --features semispace,sanity

# Build different implementations of heap layout
cargo build --features nogc
cargo build --features nogc,force_32bit_heap_layout

arch=`rustc --print cfg | grep target_arch | cut -f2 -d"\""`
os=`rustc --print cfg | grep target_os | cut -f2 -d"\""`
# For x86_64-linux, also see if we can build for i686
if [[ $arch == "x86_64" && $os == "linux" ]]; then
    cargo build --target i686-unknown-linux-gnu --features nogc
    cargo build --target i686-unknown-linux-gnu --features nogc,force_32bit_heap_layout
fi