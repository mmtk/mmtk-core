#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include "mmtk.h"

int main() {
    volatile uint64_t * tmp;
    gc_init(1024*1024*1024);
    for (int i=0; i<1024*1024*100; i++) {
        tmp = alloc(8, 1, 0);
        #ifdef STORE
            *tmp = 42;
        #endif
    }
}
