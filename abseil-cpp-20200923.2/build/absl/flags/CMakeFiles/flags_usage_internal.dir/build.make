# CMAKE generated file: DO NOT EDIT!
# Generated by "Unix Makefiles" Generator, CMake Version 3.19

# Delete rule output on recipe failure.
.DELETE_ON_ERROR:


#=============================================================================
# Special targets provided by cmake.

# Disable implicit rules so canonical targets will work.
.SUFFIXES:


# Disable VCS-based implicit rules.
% : %,v


# Disable VCS-based implicit rules.
% : RCS/%


# Disable VCS-based implicit rules.
% : RCS/%,v


# Disable VCS-based implicit rules.
% : SCCS/s.%


# Disable VCS-based implicit rules.
% : s.%


.SUFFIXES: .hpux_make_needs_suffix_list


# Command-line flag to silence nested $(MAKE).
$(VERBOSE)MAKESILENT = -s

#Suppress display of executed commands.
$(VERBOSE).SILENT:

# A target that is always out of date.
cmake_force:

.PHONY : cmake_force

#=============================================================================
# Set environment variables for the build.

# The shell in which to execute make rules.
SHELL = /bin/sh

# The CMake executable.
CMAKE_COMMAND = /usr/bin/cmake

# The command to remove a file.
RM = /usr/bin/cmake -E rm -f

# Escaping for special characters.
EQUALS = =

# The top-level source directory on which CMake was run.
CMAKE_SOURCE_DIR = /home/paiger/mmtk-core/abseil-cpp-20200923.2

# The top-level build directory on which CMake was run.
CMAKE_BINARY_DIR = /home/paiger/mmtk-core/abseil-cpp-20200923.2/build

# Include any dependencies generated for this target.
include absl/flags/CMakeFiles/flags_usage_internal.dir/depend.make

# Include the progress variables for this target.
include absl/flags/CMakeFiles/flags_usage_internal.dir/progress.make

# Include the compile flags for this target's objects.
include absl/flags/CMakeFiles/flags_usage_internal.dir/flags.make

absl/flags/CMakeFiles/flags_usage_internal.dir/internal/usage.cc.o: absl/flags/CMakeFiles/flags_usage_internal.dir/flags.make
absl/flags/CMakeFiles/flags_usage_internal.dir/internal/usage.cc.o: ../absl/flags/internal/usage.cc
	@$(CMAKE_COMMAND) -E cmake_echo_color --switch=$(COLOR) --green --progress-dir=/home/paiger/mmtk-core/abseil-cpp-20200923.2/build/CMakeFiles --progress-num=$(CMAKE_PROGRESS_1) "Building CXX object absl/flags/CMakeFiles/flags_usage_internal.dir/internal/usage.cc.o"
	cd /home/paiger/mmtk-core/abseil-cpp-20200923.2/build/absl/flags && /usr/bin/c++ $(CXX_DEFINES) $(CXX_INCLUDES) $(CXX_FLAGS) -o CMakeFiles/flags_usage_internal.dir/internal/usage.cc.o -c /home/paiger/mmtk-core/abseil-cpp-20200923.2/absl/flags/internal/usage.cc

absl/flags/CMakeFiles/flags_usage_internal.dir/internal/usage.cc.i: cmake_force
	@$(CMAKE_COMMAND) -E cmake_echo_color --switch=$(COLOR) --green "Preprocessing CXX source to CMakeFiles/flags_usage_internal.dir/internal/usage.cc.i"
	cd /home/paiger/mmtk-core/abseil-cpp-20200923.2/build/absl/flags && /usr/bin/c++ $(CXX_DEFINES) $(CXX_INCLUDES) $(CXX_FLAGS) -E /home/paiger/mmtk-core/abseil-cpp-20200923.2/absl/flags/internal/usage.cc > CMakeFiles/flags_usage_internal.dir/internal/usage.cc.i

absl/flags/CMakeFiles/flags_usage_internal.dir/internal/usage.cc.s: cmake_force
	@$(CMAKE_COMMAND) -E cmake_echo_color --switch=$(COLOR) --green "Compiling CXX source to assembly CMakeFiles/flags_usage_internal.dir/internal/usage.cc.s"
	cd /home/paiger/mmtk-core/abseil-cpp-20200923.2/build/absl/flags && /usr/bin/c++ $(CXX_DEFINES) $(CXX_INCLUDES) $(CXX_FLAGS) -S /home/paiger/mmtk-core/abseil-cpp-20200923.2/absl/flags/internal/usage.cc -o CMakeFiles/flags_usage_internal.dir/internal/usage.cc.s

# Object files for target flags_usage_internal
flags_usage_internal_OBJECTS = \
"CMakeFiles/flags_usage_internal.dir/internal/usage.cc.o"

# External object files for target flags_usage_internal
flags_usage_internal_EXTERNAL_OBJECTS =

absl/flags/libabsl_flags_usage_internal.a: absl/flags/CMakeFiles/flags_usage_internal.dir/internal/usage.cc.o
absl/flags/libabsl_flags_usage_internal.a: absl/flags/CMakeFiles/flags_usage_internal.dir/build.make
absl/flags/libabsl_flags_usage_internal.a: absl/flags/CMakeFiles/flags_usage_internal.dir/link.txt
	@$(CMAKE_COMMAND) -E cmake_echo_color --switch=$(COLOR) --green --bold --progress-dir=/home/paiger/mmtk-core/abseil-cpp-20200923.2/build/CMakeFiles --progress-num=$(CMAKE_PROGRESS_2) "Linking CXX static library libabsl_flags_usage_internal.a"
	cd /home/paiger/mmtk-core/abseil-cpp-20200923.2/build/absl/flags && $(CMAKE_COMMAND) -P CMakeFiles/flags_usage_internal.dir/cmake_clean_target.cmake
	cd /home/paiger/mmtk-core/abseil-cpp-20200923.2/build/absl/flags && $(CMAKE_COMMAND) -E cmake_link_script CMakeFiles/flags_usage_internal.dir/link.txt --verbose=$(VERBOSE)

# Rule to build all files generated by this target.
absl/flags/CMakeFiles/flags_usage_internal.dir/build: absl/flags/libabsl_flags_usage_internal.a

.PHONY : absl/flags/CMakeFiles/flags_usage_internal.dir/build

absl/flags/CMakeFiles/flags_usage_internal.dir/clean:
	cd /home/paiger/mmtk-core/abseil-cpp-20200923.2/build/absl/flags && $(CMAKE_COMMAND) -P CMakeFiles/flags_usage_internal.dir/cmake_clean.cmake
.PHONY : absl/flags/CMakeFiles/flags_usage_internal.dir/clean

absl/flags/CMakeFiles/flags_usage_internal.dir/depend:
	cd /home/paiger/mmtk-core/abseil-cpp-20200923.2/build && $(CMAKE_COMMAND) -E cmake_depends "Unix Makefiles" /home/paiger/mmtk-core/abseil-cpp-20200923.2 /home/paiger/mmtk-core/abseil-cpp-20200923.2/absl/flags /home/paiger/mmtk-core/abseil-cpp-20200923.2/build /home/paiger/mmtk-core/abseil-cpp-20200923.2/build/absl/flags /home/paiger/mmtk-core/abseil-cpp-20200923.2/build/absl/flags/CMakeFiles/flags_usage_internal.dir/DependInfo.cmake --color=$(COLOR)
.PHONY : absl/flags/CMakeFiles/flags_usage_internal.dir/depend

