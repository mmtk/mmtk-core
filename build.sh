#!/bin/bash

clang -lmmtk -Ltarget/debug -o main -Iapi ./api/main.c
LD_LIBRARY_PATH=target/debug ./main

clang -lmmtk -Ltarget/release -o main -Iapi ./api/main.c
LD_LIBRARY_PATH=target/release ./main
