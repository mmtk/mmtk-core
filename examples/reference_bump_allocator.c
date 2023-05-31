#include <stdlib.h>
#include <stdio.h>
#include <sys/mman.h>
#include "../api/mmtk.h"

typedef struct {
    void* heap_start;
    void* heap_end;
    void* heap_cursor;
} Space;

Space IMMORTAL_SPACE;

size_t align_up (size_t addr, size_t align) {
    return (addr + align - 1) & ~(align - 1);
}

extern void gc_init(size_t heap_size) {
    size_t SPACE_ALIGN = 1 << 19;
    void* alloced = mmap(NULL, heap_size + SPACE_ALIGN, PROT_READ|PROT_WRITE|PROT_EXEC, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0);
    if (!alloced) {
        printf("Unable to allocate memory\n");
        exit(1);
    }
    IMMORTAL_SPACE.heap_start = (void*) align_up((size_t) alloced, SPACE_ALIGN);
    IMMORTAL_SPACE.heap_end = (void*) ((size_t) IMMORTAL_SPACE.heap_start + heap_size);
    IMMORTAL_SPACE.heap_cursor = IMMORTAL_SPACE.heap_start;
}

extern MMTk_Mutator bind_mutator(void *tls) {
    return NULL;
}

extern void* align_allocation(void* region, size_t align, size_t offset) {
    ssize_t region_signed = (ssize_t) region;

    ssize_t mask = (ssize_t) (align - 1);
    ssize_t neg_off = -offset;
    ssize_t delta = (neg_off - region_signed) & mask;

    return (void*) ((ssize_t)region + delta);
}

extern void* alloc(MMTk_Mutator mutator, size_t size,
                   size_t align, size_t offset, int allocator) {

    void* result = align_allocation(IMMORTAL_SPACE.heap_cursor, align, offset);
    void* new_cursor = (void*)((size_t) result + size);
    if (new_cursor > IMMORTAL_SPACE.heap_end) {
        return NULL;
    }
    IMMORTAL_SPACE.heap_cursor = new_cursor;
    return (void*) result;
}

extern void* alloc_slow(MMTk_Mutator mutator, size_t size,
                        size_t align, size_t offset, int allocator) {

    perror("Not implemented\n");
    exit(1);
    return NULL;
}

void* mmtk_malloc(size_t size) {
    return alloc(NULL, size, 1, 0, 0);
}

void mmtk_free(void* ptr) {
    return;
}