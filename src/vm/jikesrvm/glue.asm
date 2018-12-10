.global jikesrvm_alloc_slow
.extern alloc_slow

jikesrvm_alloc_slow:
    pushl 0x100(%esi)
    mov %esp, 0x100(%esi)
    pushl $0xffffffff #method id
    pushl $42 #fingerprint
    push %ebx
    push %edi
    push %ebp
    pushl 0x2c(%esp)
    pushl 0x2c(%esp)
    pushl 0x2c(%esp)
    pushl 0x2c(%esp)
    pushl 0x2c(%esp)
    call alloc_slow
    add $0x14, %esp #shrink stack for args
    popl %ebp
    popl %edi
    popl %ebx
    add $0x8, %esp #shrink stack for method id
    popl 0x100(%esi)
    ret
