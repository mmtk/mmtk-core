#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include "mmtk.h"

int main() {
    volatile uint64_t * tmp;
    mmtk_set_heap_size(1024*1024*1024);
    mmtk_gc_init();
    MMTk_Mutator handle = mmtk_bind_mutator(0);

    for (int i=0; i<1024*1024*100; i++) {
        tmp = mmtk_alloc(handle, 8, 1, 0, 0);
        #ifdef STORE
            *tmp = 42;
        #endif
    }
}
