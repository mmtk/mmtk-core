# Note: cargo-rustc is influenced by the environment variable CARGO_BUILD_TARGET
# which is specified in minimal-tests-core.yml
arch=`cargo rustc -- --print cfg | grep target_arch | cut -f2 -d"\""`
os=`cargo rustc -- --print cfg | grep target_os | cut -f2 -d"\""`

project_root=$(dirname "$0")/../..

cargo_toml=$project_root/Cargo.toml

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

# Get all non exclusive features
init_non_exclusive_features() {
    declare -a features=()
    parse_features=false
    i=0

    while IFS= read -r line; do
        # Only parse non mutally exclusive features
        if [[ $line == *"-- Non mutually exclusive features --"* ]]; then
            parse_features=true
            continue
        fi
        if [[ $line == *"-- Mutally exclusive features --"* ]]; then
            parse_features=false
            continue
        fi

        # Skip other comment lines
        if [[ $line == \#* ]]; then
            continue
        fi

        if $parse_features ; then
            # Get feature name before '='
            IFS='='; feature=($line); unset IFS;
            if [[ ! -z "$feature" ]]; then
                # Trim whitespaces
                features[i]=$(echo $feature)
                let "i++"
            fi
        fi
    done < $cargo_toml

    non_exclusive_features=$(IFS=$','; echo "${features[*]}")
}

# Get exclusive features
init_exclusive_features() {
    parse_features=false
    i=0
    
    # Current group
    group=
    # Group index
    gi=0
    # Features in the current group
    declare -a features=()

    while IFS= read -r line; do
        # Only parse mutally exclusive features
        if [[ $line == *"-- Mutally exclusive features --"* ]]; then
            parse_features=true
            continue
        fi

        # Start a new group
        if [[ $line == *"Group:"* ]]; then
            # Save current group, and clear current features
            if [[ ! -z "$group" ]]; then
                exclusive_features[gi]=$(echo $group:)$(IFS=$',';echo "${features[*]}")
                let "gi++"
                features=()
            fi
            # Extract group name
            group=$(echo $line | cut -c9-)
        fi

        # Skip other comment lines
        if [[ $line == \#* ]]; then
            continue
        fi

        if $parse_features ; then
            # Get feature name before '='
            IFS='='; feature=($line); unset IFS;
            if [[ ! -z "$feature" ]]; then
                # Trim whitespaces
                features[i]=$(echo $feature)
                let "i++"
            fi
        fi
    done < $cargo_toml
}

# non mutally exclusive features
non_exclusive_features=
init_non_exclusive_features
# mutally exclusive features
exclusive_features=()
init_exclusive_features

set -xe
