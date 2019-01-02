use ::util::Address;

pub trait Memory {
  fn dzmmap(start: Address, size: usize) -> i32;
}
