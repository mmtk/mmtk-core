#include <stdio.h>
#include "mmtk.h"

int main(int argc, char* argv[]){
    gc_init(1024*1024);

    MMTk_Mutator handle = bind_mutator(0);
    
    for (int i=0;i<4;i++){
        int arr_size = 10000;
        int* my_arr = alloc(handle, sizeof(int)*arr_size, 8, -4, 0);
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
