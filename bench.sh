#!/bin/bash

echo "Using malloc"
clang -O3 -lmmtk -Ltarget/release -o bench-exe -Iapi ./bench/bench.c
export LD_LIBRARY_PATH=target/release
time ./bench-exe

echo "Using Rust bump pointer allocator"
clang -O3 -lmmtk -Ltarget/release -o bench-exe -D TEST -Iapi ./bench/bench.c
export LD_LIBRARY_PATH=target/release
time ./bench-exe

echo "Using C bump pointer allocator"
clang -O3 -shared -fPIC -o bench/libmmtk.so bench/bump_allocator.c
clang -O3 -lmmtk -Lbench -o bench-exe -D TEST -Iapi ./bench/bench.c
export LD_LIBRARY_PATH=bench
time ./bench-exe
