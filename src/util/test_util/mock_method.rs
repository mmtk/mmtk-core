use std::any::Any;

pub trait MockAny {
    fn call_any(&mut self, args: Box<dyn Any>) -> Box<dyn Any>;
}

impl<I: 'static, R: 'static> MockAny for MockMethod<I, R> {
    fn call_any(&mut self, args: Box<dyn Any>) -> Box<dyn Any> {
        let typed_args: Box<I> = args.downcast().unwrap();
        let typed_args_inner: I = *typed_args;
        let typed_ret = self.call(typed_args_inner);
        Box::new(typed_ret)
    }
}

pub struct MockMethod<I, R> {
    imp: MockImpl<I, R>,
}

pub enum MockImpl<I, R> {
    Sequence(Vec<MockClosure<I, R>>),
    Fixed(MockClosure<I, R>),
}

pub type MockClosureSignature<I, R> = Box<dyn Fn(I) -> R + Send + Sync>;

pub struct MockClosure<I, R> {
    closure: MockClosureSignature<I, R>,
    call_count: usize,
}

impl<I, R> MockClosure<I, R> {
    fn new(closure: MockClosureSignature<I, R>) -> Self {
        Self {
            closure,
            call_count: 0,
        }
    }
    fn call(&mut self, args: I) -> R {
        self.call_count += 1;
        (self.closure)(args)
    }
}

impl<I, R> std::default::Default for MockMethod<I, R> {
    fn default() -> Self {
        Self::new_unimplemented()
    }
}

impl<I, R> MockMethod<I, R> {
    pub fn new_unimplemented() -> Self {
        Self {
            imp: MockImpl::Fixed(MockClosure::new(Box::new(|_| unimplemented!()))),
        }
    }

    pub fn new_default() -> Self
    where
        R: Default,
    {
        Self {
            imp: MockImpl::Fixed(MockClosure::new(Box::new(|_| R::default()))),
        }
    }

    pub fn new_fixed(closure: MockClosureSignature<I, R>) -> Self {
        Self {
            imp: MockImpl::Fixed(MockClosure::new(closure)),
        }
    }

    pub fn new_sequence(closures: Vec<MockClosureSignature<I, R>>) -> Self {
        Self {
            imp: MockImpl::Sequence(closures.into_iter().map(|c| MockClosure::new(c)).collect()),
        }
    }

    pub fn call(&mut self, args: I) -> R {
        let cur_call = self.call_count();

        match &mut self.imp {
            MockImpl::Sequence(closures) => {
                let len = closures.len();
                closures[cur_call % len].call(args)
            }
            MockImpl::Fixed(closure) => closure.call(args),
        }
    }

    pub fn is_called(&self) -> bool {
        self.call_count() > 0
    }

    pub fn call_count(&self) -> usize {
        match &self.imp {
            MockImpl::Fixed(c) => c.call_count,
            MockImpl::Sequence(vec) => vec.iter().map(|c| c.call_count).sum(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_fixed_single_arg() {
        let mut mock = MockMethod::new_fixed(Box::new(|a: usize| -> usize { a + 1 }));
        assert_eq!(mock.call_count(), 0);
        let ret = mock.call(0);
        assert_eq!(ret, 1);
        assert_eq!(mock.call_count(), 1);
    }

    #[test]
    fn mock_fixed_multi_args() {
        let mut mock = MockMethod::new_fixed(Box::new(|(a, b): (usize, usize)| -> usize { a + b }));
        assert_eq!(mock.call_count(), 0);
        let ret = mock.call((1, 1));
        assert_eq!(ret, 2);
        assert_eq!(mock.call_count(), 1);
    }

    #[test]
    fn mock_fixed_no_arg() {
        let mut mock = MockMethod::new_fixed(Box::new(|()| -> usize { 42 }));
        assert_eq!(mock.call_count(), 0);
        let ret = mock.call(());
        assert_eq!(ret, 42);
        assert_eq!(mock.call_count(), 1);
    }

    #[test]
    fn mock_sequence() {
        let mut mock = MockMethod::new_sequence(vec![
            Box::new(|()| -> usize { 0 }),
            Box::new(|()| -> usize { 1 }),
        ]);
        assert_eq!(mock.call_count(), 0);

        assert_eq!(mock.call(()), 0);
        assert_eq!(mock.call_count(), 1);
        assert_eq!(mock.call(()), 1);
        assert_eq!(mock.call_count(), 2);

        assert_eq!(mock.call(()), 0);
        assert_eq!(mock.call_count(), 3);
        assert_eq!(mock.call(()), 1);
        assert_eq!(mock.call_count(), 4);
    }
}
