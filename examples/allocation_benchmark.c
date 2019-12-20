#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include "mmtk.h"

int main() {
    volatile uint64_t * tmp;
    gc_init(1024*1024*1024);
    MMTk_Mutator handle = bind_mutator(0);

    for (int i=0; i<1024*1024*100; i++) {
        tmp = alloc(handle, 8, 1, 0, 0);
        #ifdef STORE
            *tmp = 42;
        #endif
    }
}
