#ifdef __cplusplus
extern "C" {
#endif

#include <stdbool.h>

typedef void* MMTk_Mutator;
typedef void* MMTk_TraceLocal;

extern void gc_init(size_t heap_size);

/**
 * Allocation
 */
extern MMTk_Mutator bind_mutator(size_t thread_id);

extern void* alloc(MMTk_Mutator mutator, size_t size,
    size_t align, ssize_t offset, int allocator);

extern void* alloc_slow(MMTk_Mutator mutator, size_t size,
    size_t align, ssize_t offset, int allocator);


/**
 * Tracing
 */
extern void report_delayed_root_edge(MMTk_TraceLocal trace_local,
                                     void* addr);

extern bool will_not_move_in_current_collection(MMTk_TraceLocal trace_local,
                                                void* obj);

extern void process_interior_edge(MMTk_TraceLocal trace_local, void* target,
                                  void* slot, bool root);


/**
 * JikesRVM-specific
 */
extern void jikesrvm_gc_init(void* jtoc, size_t heap_size);

extern void enable_collection(size_t size);

extern void start_control_collector(size_t thread_id);

extern void start_worker(size_t thread_id, void* worker);

#ifdef __cplusplus
}
#endif
