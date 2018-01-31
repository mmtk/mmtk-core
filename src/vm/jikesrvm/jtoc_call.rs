#[cfg(target_arch = "x86")]
#[macro_export]
macro_rules! jtoc_call {
    ($offset:ident, $thread_id:expr $(, $arg:ident)*) => ({
        let call_addr = (JTOC_BASE + $offset).load::<fn()>();
        jikesrvm_call!(call_addr, $thread_id $(, $arg)*)
    });
}

#[cfg(target_arch = "x86")]
#[macro_export]
macro_rules! jikesrvm_instance_call {
    ($obj:expr, $offset:expr, $thread_id:expr $(, $arg:ident)*) => ({
        let call_addr = ($obj + $offset).load::<fn()>();
        jikesrvm_call!(call_addr, $thread_id $(, $arg)*)
    });
}

#[cfg(target_arch = "x86")]
#[macro_export]
macro_rules! jikesrvm_call {
    ($call_addr:expr, $thread_id:expr $(, $arg:ident)*) => ({
        use ::vm::jikesrvm::collection::VMCollection as _VMCollection;

        let ret: usize;
        let rvm_thread = _VMCollection::thread_from_id($thread_id).as_usize();

        jikesrvm_call_args!($($arg),*);

        asm!("call *%ebx" : "={eax}"(ret) : "{esi}"(rvm_thread),
             "{ebx}"($call_addr) : "eax", "ebx", "ecx", "edx", "esi", "memory");

        ret
    });
}

#[cfg(target_arch = "x86")]
macro_rules! jikesrvm_call_args {
    () => ();

    ($arg1:ident) => (
        asm!("push %eax" : : "{eax}"($arg1) : "sp", "memory");
    );

    ($arg1:ident, $arg2:ident) => (
        jikesrvm_call_args!($arg1);
        asm!("push %edx" : : "{edx}"($arg2) : "sp", "memory");
    );

    ($arg1:ident, $arg2:ident, $($arg:ident),+) => (
        jikesrvm_call_args!($arg1, $arg2);
        $(
            asm!("push %ebx" : : "{ebx}"($arg) : "sp", "memory");
        )*
    );
}