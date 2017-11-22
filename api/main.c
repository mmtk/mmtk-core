#include <stdio.h>
#include "mmtk.h"

int main(int argc, char* argv[]){
    gc_init(1024*1024*1024);
    void* my_lovely_heap = alloc(1024, 8); 
    printf("%p\n", my_lovely_heap);
    while(1);
    return 0;
}
