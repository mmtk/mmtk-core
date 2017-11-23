#!/bin/bash

clang -lmmtk -Ltarget/release -o bench -Iapi ./api/bench.c
export LD_LIBRARY_PAT=target/release
time ./bench
