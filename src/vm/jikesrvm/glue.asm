.global jikesrvm_alloc_slow, jikesrvm_handle_user_collection_request
.extern alloc_slow, handle_user_collection_request

jikesrvm_alloc_slow:
    pushl 0x100(%esi)
    mov %esp, 0x100(%esi)
    pushl $0xffffffff #method id
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    call alloc_slow
    add $0x18, %esp #shrink stack for args and method id
    popl 0x100(%esi)
    ret

jikesrvm_handle_user_collection_request:
    pushl 0x100(%esi)
    mov %esp, 0x100(%esi)
    pushl $0xffffffff #method id
    pushl 0xc(%esp)
    call handle_user_collection_request
    add $0x8, %esp #shrink stack for args and method id
    popl 0x100(%esi)
    ret
