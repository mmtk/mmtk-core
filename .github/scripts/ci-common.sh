arch=`rustc --print cfg | grep target_arch | cut -f2 -d"\""`
os=`rustc --print cfg | grep target_os | cut -f2 -d"\""`