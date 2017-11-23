#!/bin/bash

echo "Using malloc"
clang -lmmtk -Ltarget/release -o bench -Iapi ./api/bench.c
export LD_LIBRARY_PAT=target/release
time ./bench

echo "Using bump point allocator"
clang -lmmtk -Ltarget/release -o bench -D TEST -Iapi ./api/bench.c
export LD_LIBRARY_PAT=target/release
time ./bench