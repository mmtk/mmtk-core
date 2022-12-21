set -xe

sudo apt-get update
sudo apt-get install build-essential gcc-multilib -y

# Necessary libraries for 32bit mmtk build
sudo dpkg --add-architecture i386
sudo apt-get update
sudo apt-get install zlib1g-dev:i386
sudo apt-get install libc6-dev-i386