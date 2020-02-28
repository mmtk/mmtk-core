#ifndef MMTK_H
#define MMTK_H

#include <stdbool.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef void* MMTk_Mutator;
typedef void* MMTk_TraceLocal;

/**
 * Allocation
 */
extern MMTk_Mutator bind_mutator(void *tls);
extern void destroy_mutator(MMTk_Mutator mutator);

extern void* alloc(MMTk_Mutator mutator, size_t size,
    size_t align, size_t offset, int allocator);

extern void* alloc_slow(MMTk_Mutator mutator, size_t size,
    size_t align, size_t offset, int allocator);

extern void post_alloc(MMTk_Mutator mutator, void* refer, void* type_refer,
    int bytes, int allocator);

extern bool is_valid_ref(void* ref);
extern bool is_mapped_object(void* ref);
extern bool is_mapped_address(void* addr);
extern void modify_check(void* ref);

/**
 * Tracing
 */
extern void report_delayed_root_edge(MMTk_TraceLocal trace_local,
                                     void* addr);

extern bool will_not_move_in_current_collection(MMTk_TraceLocal trace_local,
                                                void* obj);

extern void process_interior_edge(MMTk_TraceLocal trace_local, void* target,
                                  void* slot, bool root);

extern void* trace_get_forwarded_referent(MMTk_TraceLocal trace_local, void* obj);

extern void* trace_get_forwarded_reference(MMTk_TraceLocal trace_local, void* obj);

extern void* trace_retain_referent(MMTk_TraceLocal trace_local, void* obj);

extern bool trace_is_live(MMTk_TraceLocal trace_local, void* obj);
extern void* trace_root_object(MMTk_TraceLocal trace_local, void* obj);

extern void process_edge(MMTk_TraceLocal trace, void* obj);

/**
 * Misc
 */
extern void gc_init(size_t heap_size);
extern bool will_never_move(void* object);
extern bool process(char* name, char* value);
extern void scan_region();
extern void handle_user_collection_request(void *tls);

extern void start_control_collector(void *tls);
extern void start_worker(void *tls, void* worker);

/**
 * VM Accounting
 */
extern size_t free_bytes();
extern size_t total_bytes();

/**
 * OpenJDK-specific
 */
typedef struct {
    void (*stop_all_mutators) (void *tls);
    void (*resume_mutators) (void *tls);
    void (*spawn_collector_thread) (void *tls, void *ctx);
    void (*block_for_gc) ();
    void* (*active_collector) (void* tls);
    void* (*get_next_mutator) ();
    void (*reset_mutator_iterator) ();
    void (*compute_static_roots) (void* trace, void* tls);
    void (*compute_global_roots) (void* trace, void* tls);
    void (*compute_thread_roots) (void* trace, void* tls);
    void (*scan_object) (void* trace, void* object, void* tls);
    void (*dump_object) (void* object);
    size_t (*get_object_size) (void* object);
    void* (*get_mmtk_mutator) (void* tls);
    bool (*is_mutator) (void* tls);
} OpenJDK_Upcalls;

extern void openjdk_gc_init(OpenJDK_Upcalls *calls, size_t heap_size);

extern size_t used_bytes();
extern void* starting_heap_address();
extern void* last_heap_address();
extern void iterator(); // ???


// (It is the total_space - capacity_of_to_space in Semispace )
// PZ: It shouldn't be ...?
extern size_t openjdk_max_capacity();
extern size_t _noaccess_prefix();  // ???
extern size_t _alignment();        // ???
extern bool   executable();

/**
 * Reference Processing
 */
extern void add_weak_candidate(void* ref, void* referent);
extern void add_soft_candidate(void* ref, void* referent);
extern void add_phantom_candidate(void* ref, void* referent);

extern void harness_begin(void *tls);
extern void harness_end();

#ifdef __cplusplus
}
#endif

#endif // MMTK_H