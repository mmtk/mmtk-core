// TODO: #1 is fixed, but we need more guarantees that this won't gratuitously break
#[cfg(target_arch = "x86")]
#[macro_export]
macro_rules! jtoc_call {
    ($offset:ident, $thread_id:expr $(, $arg:ident)*) => (unsafe {
        let ret: usize;
        let call_addr = (JTOC_BASE + $offset).load::<fn()>();
        let rvm_thread
        = Address::from_usize(((JTOC_BASE + THREAD_BY_SLOT_FIELD_JTOC_OFFSET)
            .load::<usize>() + 4 * $thread_id)).load::<usize>();

        jtoc_args!($($arg),*);

        asm!("call ebx" : "={eax}"(ret) : "{esi}"(rvm_thread),
             "{ebx}"(call_addr) : "eax", "ebx", "ecx", "edx", "esi", "memory" : "intel");

        ret
    });
}

#[cfg(target_arch = "x86")]
macro_rules! jtoc_args {
    () => ();

    ($arg1:ident) => (
        asm!("push eax" : : "{eax}"($arg1) : "sp", "memory" : "intel");
    );

    ($arg1:ident, $arg2:ident) => (
        jtoc_args!($arg1);
        asm!("push edx" : : "{edx}"($arg2) : "sp", "memory" : "intel");
    );

    ($arg1:ident, $arg2:ident, $($arg:ident),+) => (
        jtoc_args!($arg1, $arg2);
        $(
            asm!("push ebx" : : "{ebx}"($arg) : "sp", "memory" : "intel");
        )*
    );
}