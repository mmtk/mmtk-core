use crate::api::*;
use crate::DummyVM;
use mmtk::util::{OpaquePointer};
use mmtk::Allocator;
use std::ptr;
use std::time::{SystemTime, UNIX_EPOCH};


struct Node { 
    _v1: i32, 
    _v2: i32,
    _left: *mut Node,
    _right: *mut Node,
}

const K_STRETCH_TREE_DEPTH: usize = 18;
const K_LONG_LIVED_TREE_DEPTH: usize = 16;
const K_ARRAY_SIZE: usize =100000;
const K_MIN_TREE_DEPTH: usize = 4;
const K_MAX_TREE_DEPTH: usize = 16;

fn main() {
    gc_init(1024*1024*1024);
    let handle: *mut mmtk::SelectedMutator<DummyVM> = bind_mutator(OpaquePointer::UNINITIALIZED);

    println!("Garbage Collector Test");
    println!("Stretching memory with a binary tree of depth {}", K_STRETCH_TREE_DEPTH);
    print_diagnostics();

    let t_start = SystemTime::now();
    let since_the_epoch_s = t_start
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");	
    let in_ms_s = since_the_epoch_s.as_millis();

    let _temporary_tree : Node = make_tree(K_STRETCH_TREE_DEPTH, handle);
		
    println!("Creating a long-lived binary tree of depth {}", K_LONG_LIVED_TREE_DEPTH);
    let ptr1 = alloc(handle, K_LONG_LIVED_TREE_DEPTH*16, 4, 0, Allocator::Default);
    let long_lived_tree = ptr1.to_mut_ptr();  
    unsafe{*long_lived_tree = Node{
        _v1: 1,
        _v2: 1,
        _left: ptr::null_mut(),
        _right: ptr::null_mut(),   
    };}
    populate(K_LONG_LIVED_TREE_DEPTH, long_lived_tree, handle);
	
    println!("Creating a long-lived array of {} doubles", K_ARRAY_SIZE);
    let ptr2 = alloc(handle, K_ARRAY_SIZE, 4, 0, Allocator::Default);
    let array = ptr2.to_mut_ptr(); 
    unsafe{*array = [0; K_ARRAY_SIZE];}
    for i in 1..=(K_ARRAY_SIZE/2+1) {
	unsafe{(*array)[i] = 1/i};
    }
    print_diagnostics();

    for i in (K_MIN_TREE_DEPTH..=K_MAX_TREE_DEPTH).step_by(2) {
	time_construction(i, handle);
    }
    if long_lived_tree == ptr::null_mut() || unsafe{(*array)[1000]} != 1/1000 {
	println!("Failed");
    }
    let t_finish = SystemTime::now();
    let since_the_epoch_f = t_finish
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");	
    let in_ms_f = since_the_epoch_f.as_millis();
    let t_elapsed = in_ms_f - in_ms_s;
    print_diagnostics();
    println!("Completed in {} ms.", t_elapsed);
}

fn tree_size(i : usize) -> usize{
     return 2_usize.pow((i+1) as u32) - 1;
}

fn number_of_iterations(i : usize) -> usize{
      return 2*tree_size(K_STRETCH_TREE_DEPTH)/tree_size(i);
}

fn populate(mut depth : usize, mut this_node : *mut Node, handle : *mut mmtk::SelectedMutator<DummyVM>) {
  if depth <= 0 {
      return;
  } else {
      depth = depth-1;
      let ptr1 = alloc(handle, depth*16, 4, 0, Allocator::Default);
      unsafe{(*this_node)._left = ptr1.to_mut_ptr();}
      populate(depth-1, unsafe{(*this_node)._left}, handle);
      let ptr2 = alloc(handle, depth*16, 4, 0, Allocator::Default);
      unsafe{(*this_node)._right = ptr2.to_mut_ptr();}
      populate(depth-1, unsafe{(*this_node)._right}, handle);
  }
}

fn make_tree(depth : usize, handle : *mut mmtk::SelectedMutator<DummyVM>) -> Node{
  let mut t_left: *mut Node = ptr::null_mut();
  let mut t_right: *mut Node = ptr::null_mut();
  if depth > 1 {unsafe{
      let ptr1 = alloc(handle, depth*16, 4, 0, Allocator::Default);
      t_left = ptr1.to_mut_ptr();
      *t_left = make_tree(depth-1, handle);
      let ptr2 = alloc(handle, depth*16, 4, 0, Allocator::Default);
      t_right = ptr2.to_mut_ptr();
      *t_right = make_tree(depth-1, handle);}
  }
  let mut _tree = Node{
    _v1: 1,
    _v2: 1,
    _left: t_left,
    _right: t_right,   
  };
  return _tree;
}

fn print_diagnostics() {
    let l_free_memory = free_bytes();
    let l_total_memory = total_bytes ();

    print!("Total memory available = {} bytes ", l_total_memory);
    println!("Free memory = {} bytes", l_free_memory);
}

fn time_construction(depth : usize, handle : *mut mmtk::SelectedMutator<DummyVM>) {
    let mut _root : Node;
    let mut t_start : SystemTime;
    let mut t_finish : SystemTime;
    let i_number_of_iterations : usize = number_of_iterations(depth);
    let mut temporary_tree: *mut Node;
    let mut ptr1 = alloc(handle, K_LONG_LIVED_TREE_DEPTH*4, 8, 0, Allocator::Default);
    temporary_tree = ptr1.to_mut_ptr();  
    unsafe{*temporary_tree = Node{
        _v1: 1,
        _v2: 1,
        _left: ptr::null_mut(),
        _right: ptr::null_mut(),   
    }}; 
    println!("Creating {} trees of depth {}", i_number_of_iterations, depth);
    t_start = SystemTime::now();
    let mut since_the_epoch_s = t_start
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");	
    let mut in_ms_s = since_the_epoch_s.as_millis();
    for _i in 1..(i_number_of_iterations+1) {
	ptr1 = alloc(handle, K_LONG_LIVED_TREE_DEPTH*16, 4, 0, Allocator::Default);
        temporary_tree = ptr1.to_mut_ptr();  
        unsafe{*temporary_tree = Node{
            _v1: 1,
            _v2: 1,
            _left: ptr::null_mut(),
            _right: ptr::null_mut(),   
        }}; 
	populate(depth, temporary_tree,handle);
    }
    t_finish = SystemTime::now();
    let mut since_the_epoch_f = t_finish
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");	
    let mut in_ms_f = since_the_epoch_f.as_millis();
    println!("Top down construction took {} msecs", (in_ms_f - in_ms_s));
    t_start = SystemTime::now();
    since_the_epoch_s = t_start
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");	
    in_ms_s = since_the_epoch_s.as_millis();
    for _i in 1..(i_number_of_iterations+1) {
        _root = make_tree(depth, handle);
    }
    t_finish = SystemTime::now();
    since_the_epoch_f = t_finish
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");	
    in_ms_f = since_the_epoch_f.as_millis();
    println!("Bottom up construction took {} msecs", (in_ms_f - in_ms_s));
}
