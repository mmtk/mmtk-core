use crate::api::*;
use crate::DummyVM;
use mmtk::util::{OpaquePointer};
use mmtk::Allocator;use std::ptr;

struct Node { 
    _v1: i32, 
    _v2: i32,
    _left: *mut Node,
    _right: *mut Node,
}

#[test]
fn run_fixed_live() {
  gc_init(200*1024*1024);  let handle = bind_mutator(OpaquePointer::UNINITIALIZED);
  let _n = create_tree(18, handle);
  alloc_loop(100000);
}

fn create_tree(depth :usize, handle : *mut mmtk::SelectedMutator<DummyVM>) -> Node{ 
  let mut t_left: *mut Node = ptr::null_mut();
  let mut t_right: *mut Node = ptr::null_mut();
  if depth > 1 {unsafe{
      let ptr1 = alloc(handle, depth*16, 8, 0, Allocator::Default);
      t_left = ptr1.to_mut_ptr();
      *t_left = create_tree(depth-1, handle);      let ptr2 = alloc(handle, depth*16, 8, 0, Allocator::Default);
      t_right = ptr2.to_mut_ptr();
      *t_right = create_tree(depth-1, handle);}
  }
  let mut _tree = Node{
    _v1: 1,
    _v2: 1,
    _left: t_left,
    _right: t_right,   
  };
  if depth > 17 {
    println!("Finished-Task1");
  }
  return _tree;
}

fn alloc_loop(mut count : i32) {
  while count > 0 {
    let mut _tree = Node{
      _v1: 1,
      _v2: 1,
      _left: ptr::null_mut(),
      _right: ptr::null_mut(),
    };
    count = count-1;
  }
  println!("Finished-Task2");
}
