#!/bin/bash

echo "Using Rust bump pointer allocator"
clang -O3 -lmmtk -L../target/release -o bench-exe -I../api ./allocation_benchmark.c
export LD_LIBRARY_PATH=../target/release
time ./bench-exe

echo "Using C bump pointer allocator"
clang -O3 -shared -fPIC -o ./libmmtk.so ./reference_bump_allocator.c
clang -O3 -lmmtk -L. -o bench-exe -I../api ./allocation_benchmark.c
export LD_LIBRARY_PATH=.
time ./bench-exe

echo "Using Rust bump pointer allocator with storing"
clang -O3 -lmmtk -L../target/release -o bench-exe -D STORE -I../api ./allocation_benchmark.c
export LD_LIBRARY_PATH=../target/release
time ./bench-exe

echo "Using C bump pointer allocator with storing"
clang -O3 -shared -fPIC -o ./libmmtk.so ./reference_bump_allocator.c
clang -O3 -lmmtk -L. -o bench-exe -D STORE -I../api ./allocation_benchmark.c
export LD_LIBRARY_PATH=.
time ./bench-exe
