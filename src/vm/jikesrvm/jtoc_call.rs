#[cfg(target_arch = "x86")]
#[macro_export]
macro_rules! jtoc_call {
    ($offset:ident, $tls:expr $(, $arg:ident)*) => ({
        let call_addr = (::vm::jikesrvm::JTOC_BASE + $offset).load::<fn()>();
        jikesrvm_call!(call_addr, $tls $(, $arg)*)
    });
}

#[cfg(target_arch = "x86")]
#[macro_export]
macro_rules! jikesrvm_instance_call {
    ($obj:expr, $offset:expr, $tls:expr $(, $arg:ident)*) => ({
        let tib = Address::from_usize(($obj + ::vm::jikesrvm::java_header::TIB_OFFSET).load::<usize>());
        let call_addr = (tib + $offset).load::<fn()>();
        jikesrvm_call!(call_addr, $tls $(, $arg)*)
    });
}

#[cfg(target_arch = "x86")]
#[macro_export]
macro_rules! jikesrvm_call {
    ($call_addr:expr, $tls:expr $(, $arg:ident)*) => ({
        use ::vm::jikesrvm::collection::VMCollection as _VMCollection;
        use libc::c_void;
        debug_assert!($tls != 0 as *mut c_void);

        let ret: usize;
        let rvm_thread = $tls;

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