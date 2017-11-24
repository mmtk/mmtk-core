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
    size_t SPACE_ALIGN = 1 << 19;
    void* alloced = malloc(heap_size + SPACE_ALIGN);
    if (!alloced) {
        printf("Unable to allocate memory\n");
        exit(1);
    }
    IMMORTAL_SPACE.heap_start = (void*) align_up((size_t) alloced, SPACE_ALIGN);
    IMMORTAL_SPACE.heap_end = (void*) ((size_t) IMMORTAL_SPACE.heap_start + heap_size);
    IMMORTAL_SPACE.heap_cursor = IMMORTAL_SPACE.heap_start;
}

void* alloc(size_t size, size_t align, size_t offset) {
    void* result = (void*) align_up((size_t) IMMORTAL_SPACE.heap_cursor, align);
    void* new_cursor = (void*)((size_t) result + size);
    if (new_cursor > IMMORTAL_SPACE.heap_end) {
        return NULL;
    }
    IMMORTAL_SPACE.heap_cursor = new_cursor;
    return result;
}
