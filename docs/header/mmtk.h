// This is an example of native API for the single instance MMTk.

// Note: the mmtk core does not directly provide this API. However, it provides
// a similar multi-instance Rust API.  A VM binding should write their own C
// header file (possibly based on this example with their own extension and
// modification), and expose the Rust API based on their native API.

#ifndef MMTK_H
#define MMTK_H

#include <stdbool.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef void* MMTk_Mutator;
typedef void* MMTk_Builder;
typedef void* MMTk;

// Initialize an MMTk instance
extern MMTk mmtk_init(MMTk_Builder builder);

// Request MMTk to create a new mutator for the given `tls` thread
extern MMTk_Mutator mmtk_bind_mutator(void* tls);

// Reclaim mutator that is no longer needed
extern void mmtk_destroy_mutator(MMTk_Mutator mutator);

// Flush mutator local state
extern void mmtk_flush_mutator(MMTk_Mutator mutator);

// Initialize MMTk scheduler and GC workers
extern void mmtk_initialize_collection(void* tls);

// Allow MMTk to perform a GC when the heap is full
extern void mmtk_enable_collection();

// Disallow MMTk to perform a GC when the heap is full
extern void mmtk_disable_collection();

// Allocate memory for an object
extern void* mmtk_alloc(MMTk_Mutator mutator,
                        size_t size,
                        size_t align,
                        size_t offset,
                        int allocator);

// Slowpath allocation for an object
extern void* mmtk_alloc_slow(MMTk_Mutator mutator,
                             size_t size,
                             size_t align,
                             size_t offset,
                             int allocator);

// Perform post-allocation hooks or actions such as initializing object metadata
extern void mmtk_post_alloc(MMTk_Mutator mutator,
                            void* refer,
                            int bytes,
                            int allocator);

// Return if the object pointed to by `ref` is live
extern bool mmtk_is_live_object(void* ref);

// Return if the object pointed to by `ref` is in mapped memory
extern bool mmtk_is_mapped_object(void* ref);

// Return if the address pointed to by `addr` is in mapped memory
extern bool mmtk_is_mapped_address(void* addr);

// Return if object pointed to by `object` will never move
extern bool mmtk_will_never_move(void* object);

// Process an MMTk option. Return true if option was processed successfully
extern bool mmtk_process(MMTk_Builder builder, char* name, char* value);

// Process MMTk options. Return true if all options were processed successfully
extern bool mmtk_process_bulk(MMTk_Builder builder, char* options);

// Sanity only. Scan heap for discrepancies and errors
extern void mmtk_scan_region();

// Request MMTk to trigger a GC. Note that this may not actually trigger a GC
extern void mmtk_handle_user_collection_request(void* tls);

// Run the main loop for the GC controller thread. Does not return
extern void mmtk_start_control_collector(void* tls, void* worker);

// Run the main loop for a GC worker. Does not return
extern void mmtk_start_worker(void* tls, void* worker);

// Return the current amount of free memory in bytes
extern size_t mmtk_free_bytes();

// Return the current amount of used memory in bytes
extern size_t mmtk_used_bytes();

// Return the current amount of total memory in bytes
extern size_t mmtk_total_bytes();

// Return the starting address of MMTk's heap
extern void* mmtk_starting_heap_address();

// Return the ending address of MMTk's heap
extern void* mmtk_last_heap_address();

// Add a reference to the list of weak references
extern void mmtk_add_weak_candidate(void* ref);

// Add a reference to the list of soft references
extern void mmtk_add_soft_candidate(void* ref);

// Add a reference to the list of phantom references
extern void mmtk_add_phantom_candidate(void* ref);

// Generic hook to allow benchmarks to be harnessed
extern void mmtk_harness_begin(void* tls);

// Generic hook to allow benchmarks to be harnessed
extern void mmtk_harness_end();

#ifdef __cplusplus
}
#endif

#endif  // MMTK_H
