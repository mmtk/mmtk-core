arch=`rustc --print cfg | grep target_arch | cut -f2 -d"\""`
os=`rustc --print cfg | grep target_os | cut -f2 -d"\""`

feature_list="vm_space,ro_space,code_space,sanity,nogc_lock_free,nogc_no_zeroing,single_worker"

# non mutally exclusive features
non_exclusive_features="vm_space,ro_space,code_space,sanity,nogc_lock_free,nogc_no_zeroing,single_worker"
# mutally exclusive features ("name:option1,option2,..." - name doesnt matter, but opition needs to match features in Cargo.toml)
exclusive_features=("malloc:malloc_mimalloc,malloc_jemalloc,malloc_hoard")

# Repeat a command for all the features. Requires the command as one argument (with double quotes)
for_all_features() {
    # without mutually exclusive features
    $1 --features $non_exclusive_features

    # for each mutually exclusive features
    for item in ${exclusive_features[@]}
    do
        unset features
        # split
        if [[ $item == *":"* ]]
        then
            # split name from features
            parse=(${item//:/ })
            name=${parse[0]}
            features=${parse[1]}
            features=(${features//,/ })
        fi

        # Loop over features
        for feature in ${features[@]}
        do
            $1 --features $non_exclusive_features,$feature
        done
    done
}