.global jikesrvm_alloc_slow, jikesrvm_handle_user_collection_request
.extern alloc_slow, handle_user_collection_request

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

jikesrvm_handle_user_collection_request:
    pushl 0x100(%esi)
    mov %esp, 0x100(%esi)
    pushl $0xffffffff #method id
    pushl $42 #fingerprint
    push %ebx
    push %edi
    push %ebp
    pushl 0x2c(%esp)
    call handle_user_collection_request
    add $0x4, %esp #shrink stack for args
    popl %ebp
    popl %edi
    popl %ebx
    add $0x8, %esp #shrink stack for method id
    popl 0x100(%esi)
    ret
