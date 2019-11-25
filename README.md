# Save3DS

[![Build Status](https://travis-ci.com/wwylele/save3ds.svg?branch=master)](https://travis-ci.com/wwylele/save3ds)

Extract, import and FUSE program for common save format for 3DS, written in rust.

This project, along with documentation, is still WIP. There are two main components in the project: the library `libsave3ds`, and the FUSE program + extract/import tool `save3ds_fuse` that builds on top of it. The FUSE feature is not available on Windows.

Both the library and the program currently supports the following operations:
 - Full filesystem operation on save data and extdata stored on NAND, on SD or standalone
 - Editing title database and tickets

Note that the supported NAND format is in unpacked cleartext filesystem. If you want to read/write on the original NAND FAT image, you need to use other tools to extract the NAND data, or map another layer of virtual filesystem (e.g. https://github.com/ihaveamac/ninfs)

TODO:
 - Complete cartridge save data support

## Build


### Unix-like
 1. install pkg-config and FUSE library.
   - Debian: `sudo apt-get install libfuse-dev pkg-config`
   - CentOS: `sudo yum install fuse-devel pkgconfig`
   - macOS: `brew cask install osxfuse && brew install pkg-config`
   - FreeBSD: `pkg install fusefs-libs pkgconf`
 2.
 ```
 cargo build
 ```

### Unix-like (no FUSE)
```
cd save3ds_fuse && cargo build --no-default-features
```

### Windows (no FUSE)

```
cargo build
```

### Tip

This AES crate this program depends on chooses hardware/software implementation at compile time. Supply compiler options `-C target-feature=+aes` to enable hardware AES feature for better performance.

## Usage

```
save3ds_fuse ARCHIVE_NAME MOUNT_PATH [MODE] [RESOURCE_PATHS] [FORMAT_PARAM]
```

You can put options in arbitrary order. The detail description of them are:

`ARCHIVE_NAME` specifies the archive to operate on. It can be one of the following:
 - `--sdsave ID`: a game save data stored on SD. `ID` is the game title ID in 16-digit hex.
 - `--sdext ID`: a game extdata stored on SD. `ID` is the extdata ID in 16-digit hex.
 - `--nandsave ID`: a system save data stored on NAND. `ID` is the save ID in 8-digit hex.
 - `--nandext ID`: a shared extdata stored on NAND. `ID` is the extdata ID in 16-digit hex.
 - `--bare FILE`: a stand-alone save data file with path `FILE`. Note that modification to this archive will result in invalid signature in the file, and you need other tools to fix the signature.
 - `--db DB_TYPE`: a title database archive. `DB_TYPE` can be one of the following:
   - `nandtitle` refers to the file `NAND:/dbs/title.db`
   - `nandimport` refers to the file `NAND:/dbs/import.db`
   - `tmptitle` refers to the file `NAND:/dbs/tmp_t.db`
   - `tmpimport` refers to the file `NAND:/dbs/tmp_i.db`
   - `sdtitle` refers to the file `SDMC:/Nintendo 3DS/<ID0>/<ID1>/dbs/title.db`
   - `sdimport` refers to the file `SDMC:/Nintendo 3DS/<ID0>/<ID1>/dbs/import.db`
   - `ticket` refers to the file `NAND:/dbs/ticket.db`
 - `--cart FILE`: (experimental) a cartridge save data file with path `FILE`. Currently only support Card1, non-N3DS-exclusive, and read-only (needs `--readonly` for mounting).

`MOUNT_PATH` is a directory to mount/extract/import the archive content

`MODE` specifies the operation mode on the archive. It can be one of the following:
 - mount mode (default). Mount the archive to `MOUNT_PATH` as a virtual filesystem, allowing browsing and editing the content. Upon unmounting, the program saves the modification. This mode is not supported on Windows.
   - with additional flag `--readonly`, the program opens the archive in read-only mode, prevents any modification operation and skips the saving at the end.
 - extract mode (`--extract`). Extracts all content of the archive to `MOUNT_PATH`.
 - import mode (`--import`). Clear the content of the archive, and import the content from `MOUNT_PATH`.
 - touch mode (`--touch`). Just open and close the archive. Useful for testing the correctness of other specified resources. No need to specify `MOUNT_PATH` in this mode.

`RESOURCE_PATHS` contains multiple supporting directories/files. Different archive types require different portion of them. It can contain any of the following:
 - `--nand DIR`: NAND root path, required by all archive types except `--bare`. However, if `--movable` is provided, this can be omitted for SD-related archives (`--db sdtitle|sdimport`, `--sdsave` and `--sdext`).
 - `--sd DIR`: SD root path, required by SD-related archives.
 - `--boot9 FILE`: the `boot9.bin` file dumped from 3DS, required by all archive types except `--bare`
 - `--otp FILE`: the `otp.bin` file dumped from 3DS, required by `--db nandtitle|nandimport|ticket`
 - `--movable FILE`: the `movable.sed` file dumped from 3DS, optionally required by SD-related archives , if `--nand` is not provided.
 - `--game FILE`: the game dumped from the cartridge in CCI format, required by cartridge save
 - `--priv FILE`: the private header dumped from the cartrdige, required by cartridge save
 - `--key FILE|HEX`: AES slot 0x2F key Y for decrypting v6.0 cartridge save

`FORMAT_PARAM` is an optional group of options in the form of `--format param1:value1,param2:value2,...`, used in conjuntion with mount mode or import mode. When the flag `--format` presents, the archive will be formatted using the given parameters before mounting/importing. This is useful for creating a completely new archives. If an archive already exists in the place, it will be deleted. The difference between `--import` and `--import --format` is that, although both clearing the content, `--import` retains the archive layout and capacity that depends on the formatting parameters, while the addition `--format` flag can change the layout and capacity.

The parameters supported by `--format` are
 - `max_dir`/`max_file`: the maximum number of directories/files. The default is `100`
 - `dir_buckets`/`file_buckets`: the bucket count of the hash table for directories/files. The default value is calculated from `max_dir`/`max_file` using the common algorithm games use.
 - `len`: only for save data archive. Limits the physical size in bytes of the save data file. The defualt is `524288` (512 KiB). For Card1 cartridge save, only `131072` (128 KiB), `524288` (512 KiB), and `1048576` (1 MiB) are allowed, are must match the cartidge chip type.
 - `block_len`: only for save data archive. The value can only be `512` or `4096`. The default is `512` for `--sdsave` and `--bare`, and `4096` for `--nandsave` and `--cart`.
 - `duplicate_data`: only for save data archive. The value can only be `true` or `false`. The default is `true`

If you want leave all parameters in default values, you can specify an empty option, e.g. `--format ""`

These parameters behave the same as those in the `fs:USER` 3DS service functions: `FormatSaveData`, `CreateSystemSaveData` and `CreateExtSaveData`. However, the `max_dir`/`max_file` specified here is two/one larger than the one in `CreateExtSaveData`, as the latter one automatically counts the required `/user`, `/boss` and `/icon`.

Title database files currently don't support `--format`.

## Example command
```bash
save3ds_fuse \
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

## Quirks and Limitations

### Directory / file name

Save data and extdata support 16-byte directory / file name, interpreted in ASCII. As it techincally supports special characters like `'/'` in the name, special mappings are implemented to display them on the host system: characters `'/'` and `'\'`, ASCII control characters, and characters beyond `0x7F` are translated to the escape sequence `\x??`, where `??` is the byte value in two-digit hex. These escaped characters will be used when displaying the directory / file name, and you can use them when editing the name. Names longer than 16-bytes are always rejected.

Prohibited characters specific to Windows are not taken care of. They are usually not used in games, but if they are unfortunately used, the program will likely crash / error out.

Files in title database archives are named with title ID in 16-digit hex. File names that contains non-hex characters or that is too long are rejected.

### Extdata file size

Due to the format design, extdata does not support resizing files natively on 3DS, nor creating files with zero size. This program works around the issue by deleting and recreating files on resizing, which is stupidly slow if the user appends a file on every write operation. Zero-size files created by this program can't be opened on 3DS either, so one needs to make sure there is no such file before importing the data back to 3DS.

One can create a file with a specific size, similar to the `CreateFile` operation on 3DS. This is done by specifying a special sequence `\+size` in the file name. For example, `a.bin\+123` creates the file `a.bin` with size of 123 bytes. This, however, doesn't comply with the expected filesystem behaviour, and breaks file name cache in browsers etc.

Because of all the mess, it is recommended to use `--import` mode instead of mount mode if you intend to modify the content of an extdata.

### Extdata filesystem structure

3DS system expects every extdata to have directories `/boss` and `/user`, and the file `/icon`. These directories and file are not automatically created when the program formats an extdata. One needs to manually create them, otherwise 3DS would likely fail to open the archive.

### Broken block of title database

Due to a bug (?) in 3DS, the last free block (128 bytes) of a title database archive (except for `ticket.db`) is broken. If the archive is almost full and data starts to be written to this block, they will not be saved.

### `Quota.dat` for NAND extdata

The format and function of the `Quota.dat` file is not fully investigated, and the program probably doesn't parse and update it properly for NAND extdata. This can potentially cause inconsistency if you modify a NAND extdata.

## License

Licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
