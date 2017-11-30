typedef void* MMTk_Handle;

extern void gc_init(size_t heap_size);

extern MMTk_Handle bind_allocator(size_t thread_id);

extern void* alloc(MMTk_Handle handle, size_t size,
    size_t align, ssize_t offset);

extern void* alloc_slow(MMTk_Handle handle, size_t size,
    size_t align, ssize_t offset);
