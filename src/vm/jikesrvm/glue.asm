.global jikesrvm_alloc_slow
.extern alloc_slow

jikesrvm_alloc_slow:
    pushl 0x100(%esi)
    mov %esp, 0x100(%esi)
    push $0xffffffff
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    call alloc_slow
    add $0x18, %esp
    popl 0x100(%esi)
    ret
