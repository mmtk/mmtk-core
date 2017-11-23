#!/bin/bash

echo "Using malloc"
clang -lmmtk -Ltarget/release -o bench -Iapi ./bench/bench.c
export LD_LIBRARY_PAT=target/release
time ./bench

echo "Using bump point allocator"
clang -lmmtk -Ltarget/release -o bench -D TEST -Iapi ./bench/bench.c
export LD_LIBRARY_PAT=target/release
time ./bench