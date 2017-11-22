#include <stdio.h>
#include "mmtk.h"

int main(int argc, char* argv[]){
    gc_init(1024*1024*1024);
    int* my_lovely_heap = alloc(1, 4);
    for (int i=0;i<1000000;i++){
        my_lovely_heap[i]=i;
    }
    printf("%d", my_lovely_heap[1000]);
    
    return 0;
}
