# Save3DS

Library and FUSE program to extract and modify common save format for 3DS, written in rust.

This project, along with documentation, is still WIP. There are two main components in the project: the library `libsave3ds`, and the FUSE program `save3ds_fuse` that builds on top of it.

The FUSE program only supports macOS and linux. I haven't added Windows support because it seems there isn't a ready-made FUSE-like library for Windows in rust yet. It shouldn't be hard to make a thin layer on top of `libsave3ds` to build a usable Windows program, but I want to leave that task till the API becomes stable.

Both the library and the FUSE program currently supports the following operations:
 - Full filesystem operation on save data stored on NAND, on SD or standalone
 - Most filesystem operation on extdata stored on NAND or on SD
 - Editing title database and tickets (not fully tested)

Note that the supported NAND format is in unpacked cleartext filesystem. If you want to read/write on the original NAND FAT image, you need to use other tools to extract the NAND data, or map another layer of FUSE (e.g. https://github.com/ihaveamac/ninfs)

TODO:
 - Extdata file creation/deletion
 - Cartridge save data support

## Example command
```bash
./save3ds_fuse \
    # Sets the path to NAND root, extracted/mounted from an NAND image.
    # For save/ext data on SD,
    # the only purpose of the NAND path is to provide movable.sed.
    # You can also provide the movable.sed file directly by
    # --movable /path/to/movable.sed
    --nand /home/wwylele/3ds-nand \

    # Sets the path to SD root.
    # This can be the direct path to the SD card mounted on PC.
    --sd /media/wwylele/6339-6261 \

    # Sets the path to the bootrom.
    # This is necessary for decryption & signing.
    --boot9 /home/wwylele/3dsbootrom/boot9.bin \

    # Informs the program that we want to mount
    # the SD save data with title ID 0004000000164800 (Pokemon Sun).
    # The ID is a 16-digit hex number
    --sdsave 0004000000164800 \

    # The target path. The directory must exist and be empty.
    # When the program is running,
    # the content of the mounted data will be shown in this directory.
    /home/wwylele/mount \

    # Optional "read-only" flag.
    # When this flag presents, all write operations are disabled.
    # Please always backup your data if you don't set this flag!
    -r
```

Please use the `-h` option for more explanation.

## Tip

This AES crate this program depends on chooses hardware/software implementation at compile time. Supply compiler options `-C target-feature=+aes` to enable hardware AES feature for better performance!

## License

Licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
