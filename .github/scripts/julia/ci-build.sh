set -xe

. $(dirname "$0")/common.sh

# Need a build_type argument
if [ $# -eq 0 ]
  then
    echo "No arguments supplied"
    exit 1
fi
# debug or release
build_type=$1
# plan to use
plan=$2
# moving vs non-moving
is_moving=$3

# helloworld.jl
HELLO_WORLD_JL=$SCRIPT_DIR/hello_world.jl

# build MMTk
build_args=""
if [ "$build_type" == "release" ]; then
    build_args=$build_args" --release"
fi

if [ "$is_moving" == "moving" ]; then
    MOVING=1
else
    MOVING=0
fi

# Just use default herustics.
unset MMTK_MIN_HSIZE_G
unset MMTK_MAX_HSIZE_G

cd $JULIA_PATH
# Clean first
make cleanall
# This builds the in-tree MMTk Julia binding from source as part of the Julia build.
cp $SCRIPT_DIR/Make.user $JULIA_PATH/
MMTK_MOVING=$MOVING MMTK_PLAN=$plan MMTK_BUILD=$build_type make
# Run hello world
$JULIA_PATH/julia $HELLO_WORLD_JL
