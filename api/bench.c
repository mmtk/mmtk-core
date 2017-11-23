#include <stdio.h>
#include <stdlib.h>
#include "mmtk.h"

//#define TEST

#ifdef TEST
    #define ALLOC(x) alloc(x, 1, 0)
    #define INIT gc_init(1024*1024*1024)
#else
    #define ALLOC(x) malloc(x)
    #define INIT __asm__("nop")
#endif

int main() {
    volatile void * tmp;
    INIT;
    for (int i=0; i<1024*1024*100; i++) {
        tmp = ALLOC(8);
        if (!tmp) {
            puts("Ran out of heap space :(\n");
        }
    }
}
