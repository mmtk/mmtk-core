#!/bin/bash

clang -lmmtk -Ltarget/debug -o main -Iapi ./api/main.c
