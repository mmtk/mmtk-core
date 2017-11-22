#include <stdio.h>
#include "mmtk.h"

int main(int argc, char* argv[]){
    gc_init();
    void* my_lovely_heap = alloc(1024, 8); 
    printf("%p", my_lovely_heap);
    return 0;
}
