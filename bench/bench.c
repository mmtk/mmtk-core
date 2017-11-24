#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include "mmtk.h"

/*
// Use mach_absolute_time on macOS
typedef uint64_t timestamp_t;
#include <mach/mach_time.h>

static inline void get_timestamp(timestamp_t* tstamp) {
    *tstamp = mach_absolute_time();
}
static double get_elapsed_time(timestamp_t* t0, timestamp_t* t1) {
    mach_timebase_info_data_t tb;
    timestamp_t elapsed;
    uint64_t elapsed_nano;

    mach_timebase_info(&tb);
    elapsed = *t1 - *t0;
    elapsed_nano = elapsed * tb.numer / tb.denom;
    return ((double)elapsed_nano) * 1e-9;
}*/

#ifdef TEST
    #define ALLOC(x) alloc(x, 1, 0)
    #define INIT gc_init(1024*1024*1024)
#else
    #define ALLOC(x) malloc(x)
    #define INIT __asm__("nop")
#endif

int main() {
    volatile uint64_t * tmp;
    INIT;
    timestamp_t t0, t1;
    get_timestamp(&t0);
    for (int i=0; i<1024*1024*100; i++) {
        tmp = ALLOC(8);
        *tmp = 42;
    }
    get_timestamp(&t1);
    printf("%lf\n", get_elapsed_time(&t0, &t1));
}
