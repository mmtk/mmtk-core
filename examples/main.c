#include <stdio.h>
#include "mmtk.h"

int main(int argc, char* argv[]){
    mmtk_set_heap_size(1024*1024);
    mmtk_gc_init();

    MMTk_Mutator handle = mmtk_bind_mutator(0);

    for (int i=0;i<4;i++){
        int arr_size = 10000;
        int* my_arr = mmtk_alloc(handle, sizeof(int)*arr_size, 8, 0, 0);
        if (!my_arr){
            printf("OOM\n");
            break;
        }
        for (int j=0;j<arr_size;j++){
            my_arr[j]=j;
        }
        for (int j=0;j<arr_size;j++){
            if (my_arr[j]!=j){
                printf("Sanity check failed\n");
            }
        }
        printf("%p\n", my_arr);
    }
    return 0;
}
