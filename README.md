# Summary

This tool is designed to copy a qemu-user binary into a chroot, as well as all
of it's shared object dependencies.

Simply give it the QEMU file, and the chroot directory, and it will copy all
of the dependencies in and hopefully put them in unique folders as to not
conflict with similar shared objects in the chroot!

