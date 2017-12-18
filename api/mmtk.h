typedef void* MMTk_Mutator;

extern void gc_init(size_t heap_size);

extern MMTk_Mutator bind_mutator(size_t thread_id);

extern void* alloc(MMTk_Mutator mutator, size_t size,
    size_t align, ssize_t offset);

extern void* alloc_slow(MMTk_Mutator mutator, size_t size,
    size_t align, ssize_t offset);

// JikesRVM specific
extern void jikesrvm_gc_init(void* jtoc, size_t heap_size);

extern void start_control_collector(size_t thread_id);
