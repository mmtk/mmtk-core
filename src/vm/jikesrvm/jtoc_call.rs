#[cfg(target_arch = "x86")]
#[macro_export]
macro_rules! jtoc_call {
    ($offset:ident, $thread_id:expr $(, $arg:ident)*) => ({
        let call_addr = (JTOC_BASE + $offset).load::<fn()>();
        jikesrvm_call!(call_addr, $thread_id $(, $arg)*)
    });
}

// FIXME: $offset is relative to the **TIB**, not the object itself
#[cfg(target_arch = "x86")]
#[macro_export]
macro_rules! jikesrvm_instance_call {
    ($obj:expr, $offset:expr, $thread_id:expr $(, $arg:ident)*) => ({
        unimplemented!();
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

        $(
            asm!("push %ebx" : : "{ebx}"($arg) : "sp", "memory");
        )*

        let call_addr = $call_addr;
        jikesrvm_call_helper!(ret, rvm_thread, call_addr $(, $arg)*);

        ret
    });
}

#[cfg(target_arch = "x86")]
macro_rules! jikesrvm_call_helper {
    ($ret:ident, $rvm_thread:ident, $call_addr:ident) => (
        asm!("call *%ebx"
             : "={eax}"($ret)
             : "{esi}"($rvm_thread), "{ebx}"($call_addr)
             : "ebx", "ecx", "edx", "esi", "memory"
             : "volatile");
    );

    ($ret:ident, $rvm_thread:ident, $call_addr:ident, $arg1:ident) => (
        asm!("call *%ebx"
             : "={eax}"($ret)
             : "{esi}"($rvm_thread), "{ebx}"($call_addr), "{eax}"($arg1)
             : "ebx", "ecx", "edx", "esi", "memory"
             : "volatile");
    );

    ($ret:ident, $rvm_thread:ident, $call_addr:ident, $arg1:ident, $arg2:ident $(, $arg:ident)*) => (
        asm!("call *%ebx"
             : "={eax}"($ret)
             : "{esi}"($rvm_thread), "{ebx}"($call_addr), "{eax}"($arg1), "{edx}"($arg2)
             : "ebx", "ecx", "edx", "esi", "memory"
             : "volatile");
    );
}