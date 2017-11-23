#include <stdlib.h>
#include <stdio.h>

typedef struct {
    void* heap_start;
    void* heap_end;
    void* heap_cursor;
} Space;

Space IMMORTAL_SPACE;

size_t align_up (size_t addr, size_t align) {
    return (addr + align - 1) & ~(align - 1);
}

void gc_init(size_t heap_size) {
    size_t SAPCE_ALIGN = 1 << 19;
    void* raw_start = malloc(heap_size);
    if (!raw_start) {
        printf("Unable to allocate memory\n");
        exit(1);
    }
    IMMORTAL_SPACE.heap_end = raw_start + heap_size;
    IMMORTAL_SPACE.heap_start = (void*) align_up((size_t) raw_start, SAPCE_ALIGN);
    IMMORTAL_SPACE.heap_cursor = IMMORTAL_SPACE.heap_start;
}

void* alloc(size_t size, size_t align, size_t offset) {
    void* old_cursor = IMMORTAL_SPACE.heap_cursor;
    void* new_cursor = (void*) align_up ((size_t) old_cursor + size, align);
    if (new_cursor > IMMORTAL_SPACE.heap_end) {
        return NULL;
    }
    IMMORTAL_SPACE.heap_cursor = new_cursor;
    return old_cursor;
}