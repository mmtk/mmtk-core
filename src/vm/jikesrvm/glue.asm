.global jikesrvm_alloc, jikesrvm_alloc_slow, jikesrvm_handle_user_collection_request, jikesrvm_harness_begin
.extern alloc, alloc_slow, handle_user_collection_request, harness_begin
.include "inc.asm"

jikesrvm_alloc:
    pushl fp_offset(%esi)
    mov %esp, fp_offset(%esi)
    pushl $0xffffffff #method id
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    call alloc
    add $0x18, %esp #shrink stack for args and method id
    popl fp_offset(%esi)
    ret

jikesrvm_alloc_slow:
    pushl fp_offset(%esi)
    mov %esp, fp_offset(%esi)
    pushl $0xffffffff #method id
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    pushl 0x1c(%esp)
    call alloc_slow
    add $0x18, %esp #shrink stack for args and method id
    popl fp_offset(%esi)
    ret

jikesrvm_handle_user_collection_request:
    pushl fp_offset(%esi)
    mov %esp, fp_offset(%esi)
    pushl $0xffffffff #method id
    pushl 0xc(%esp)
    call handle_user_collection_request
    add $0x8, %esp #shrink stack for args and method id
    popl fp_offset(%esi)
    ret

jikesrvm_harness_begin:
    pushl fp_offset(%esi)
    mov %esp, fp_offset(%esi)
    pushl $0xffffffff #method id
    pushl 0xc(%esp)
    call harness_begin
    add $0x8, %esp #shrink stack for args and method id
    popl fp_offset(%esi)
    ret

