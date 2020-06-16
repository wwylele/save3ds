use crate::error::*;
use crate::misc::*;
use crate::random_access_file::*;
use byte_struct::*;
use log::*;
use std::cell::Cell;
use std::rc::Rc;

bitfields!(
    #[derive(PartialEq, Clone)]
    EntryHalf: u32 {
        index: 31,
        flag: 1,
    }
);

#[derive(ByteStruct, PartialEq, Clone)]
#[byte_struct_le]
struct Entry {
    u: EntryHalf,
    v: EntryHalf,
}

/// A file allocation table with ninty flavor.
pub struct Fat {
    table: Rc<dyn RandomAccessFile>,
    data: Rc<dyn RandomAccessFile>,
    block_len: usize,
    free_blocks: Cell<usize>,
}

struct BlockMap {
    block_index: usize,
    node_start_index: usize,
}

struct Node {
    size: usize,
    prev: Option<usize>,
    next: Option<usize>,
}

fn index_bad_to_good(index: u32) -> Option<usize> {
    if index == 0 {
        None
    } else {
        Some(index as usize - 1)
    }
}

fn index_good_to_bad(index: Option<usize>) -> u32 {
    index.map_or(0, |i| i as u32 + 1)
}

fn get_node(table: &dyn RandomAccessFile, index: usize) -> Result<Node, Error> {
    let node_start: Entry = read_struct(table, (index + 1) * Entry::BYTE_LEN)?;
    if (node_start.u.flag == 1) != (node_start.u.index == 0) {
        error!("Node has broken entry");
        return make_error(Error::BrokenFat);
    }

    let size = if node_start.v.flag == 1 {
        let start_i = index + 2;
        let expand_start: Entry = read_struct(table, start_i * Entry::BYTE_LEN)?;

        if expand_start.u.flag == 0
            || expand_start.v.flag == 1
            || expand_start.u.index as usize != index + 1
        {
            error!("Expanded node has broken starting entry");
            return make_error(Error::BrokenFat);
        }

        let end_i = expand_start.v.index as usize;
        let expand_end: Entry = read_struct(table, end_i * Entry::BYTE_LEN)?;

        if expand_start != expand_end {
            error!("Expanded node has broken end entry");
            return make_error(Error::BrokenFat);
        }
        (expand_start.v.index - expand_start.u.index + 1) as usize
    } else {
        1
    };
    Ok(Node {
        size,
        prev: index_bad_to_good(node_start.u.index),
        next: index_bad_to_good(node_start.v.index),
    })
}

fn set_node(table: &dyn RandomAccessFile, index: usize, node: Node) -> Result<(), Error> {
    let node_start = Entry {
        u: EntryHalf {
            flag: if node.prev.is_none() { 1 } else { 0 },
            index: index_good_to_bad(node.prev),
        },
        v: EntryHalf {
            flag: if node.size != 1 { 1 } else { 0 },
            index: index_good_to_bad(node.next),
        },
    };
    write_struct(table, (index + 1) * Entry::BYTE_LEN, node_start)?;

    if node.size != 1 {
        let expand = Entry {
            u: EntryHalf {
                flag: 1,
                index: index_good_to_bad(Some(index)),
            },
            v: EntryHalf {
                flag: 0,
                index: index_good_to_bad(Some(index + node.size - 1)),
            },
        };
        // The index math is crazy here
        write_struct(table, (index + 2) * Entry::BYTE_LEN, expand.clone())?;
        write_struct(table, (index + node.size) * Entry::BYTE_LEN, expand)?;
    }

    Ok(())
}

fn get_head(table: &dyn RandomAccessFile) -> Result<Option<usize>, Error> {
    let head: Entry = read_struct(table, 0)?;
    if head.u.index != 0 || head.u.flag != 0 || head.v.flag != 0 {
        error!("FAT has broken head");
        return make_error(Error::BrokenFat);
    }
    Ok(index_bad_to_good(head.v.index))
}

fn set_head(table: &dyn RandomAccessFile, index: Option<usize>) -> Result<(), Error> {
    let head = Entry {
        u: EntryHalf { flag: 0, index: 0 },
        v: EntryHalf {
            flag: 0,
            index: index_good_to_bad(index),
        },
    };
    write_struct(table, 0, head)
}

// Takes some blocks from free blocks. The first allocated node has prev=None
// Precondition: there are sufficent free blocks
fn allocate(table: &dyn RandomAccessFile, mut block_count: usize) -> Result<Vec<BlockMap>, Error> {
    let mut block_list = Vec::with_capacity(block_count);

    let mut cur = get_head(table)?.unwrap();

    loop {
        let mut node = get_node(table, cur)?; // get the front free node
        if node.size <= block_count {
            // if we require no less than the current node
            // take the entire current node
            for i in cur..cur + node.size {
                block_list.push(BlockMap {
                    block_index: i,
                    node_start_index: cur,
                });
            }

            block_count -= node.size;

            if block_count == 0 {
                // if we have got just enough blocks
                if let Some(next) = node.next {
                    // if this node is not the last free node
                    // disconnect with the next free node
                    let mut next_node = get_node(table, next)?;
                    next_node.prev = None;
                    set_node(table, next, next_node)?;
                }

                // make the next free node (or None) as front
                set_head(table, node.next)?;

                // mark the last allocated node as the end
                node.next = None;
                set_node(table, cur, node)?;
                break; // and we are done
            }

            // iterate to the next free node
            cur = node.next.unwrap();
        } else {
            // if we need less than the current node
            // we need to split the current node

            // the half we are taking
            let left = Node {
                size: block_count, // take only what we need
                prev: node.prev,   // keep the prev
                next: None,        // this is the last node
            };

            // the half we are leaving
            let right = Node {
                size: node.size - block_count, // size after we take
                prev: None,                    // this node become the first free node
                next: node.next,
            };

            // update the split nodes
            set_node(table, cur, left)?;
            set_node(table, cur + block_count, right)?;

            // also update the prev index of the next node
            if let Some(next) = node.next {
                let mut next_node = get_node(table, next)?;
                if next_node.prev != Some(cur) {
                    error!("FAT has less space than it should");
                    return make_error(Error::BrokenFat);
                }
                next_node.prev = Some(cur + block_count);
                set_node(table, next, next_node)?;
            }

            // and set the second half as the front free node
            set_head(table, Some(cur + block_count))?;

            // take the first half
            for i in cur..cur + block_count {
                block_list.push(BlockMap {
                    block_index: i,
                    node_start_index: cur,
                });
            }
            break; // and we are done
        }
    }

    Ok(block_list)
}

// Frees a block list.
// Precondition: the first node has prev=None, block_list contain well-formed node.
// Remember to modify the first node if this list is split from a larger list!
fn free(table: &dyn RandomAccessFile, block_list: &[BlockMap]) -> Result<(), Error> {
    let last_node_index = block_list.last().unwrap().node_start_index;
    let maybe_free_front_index = get_head(table)?;
    if let Some(free_front_index) = maybe_free_front_index {
        let mut free_front = get_node(table, free_front_index)?;
        if free_front.prev.is_some() {
            error!("Trying to free a block list from middle");
            return make_error(Error::BrokenFat);
        }
        free_front.prev = Some(last_node_index);
        set_node(table, free_front_index, free_front)?;
    }

    let mut last_node = get_node(table, last_node_index)?;
    if last_node.next.is_some() {
        error!("Trying to free a block list that ends too early");
        return make_error(Error::BrokenFat);
    }
    last_node.next = maybe_free_front_index;
    set_node(table, last_node_index, last_node)?;
    set_head(table, Some(block_list[0].block_index))?;
    Ok(())
}

fn iterate_fat_entry(
    table: &dyn RandomAccessFile,
    first_entry: usize,
    mut callback: impl FnMut(usize, usize),
) -> Result<(), Error> {
    let mut cur_entry = Some(first_entry);
    let mut prev = None;

    while let Some(cur) = cur_entry {
        let node = get_node(table, cur)?;
        if node.prev != prev {
            error!("Inconsistent prev pointer detected while iterating");
            return make_error(Error::BrokenFat);
        }

        callback(cur, node.size);
        cur_entry = node.next;
        prev = Some(cur);
    }

    Ok(())
}

impl Fat {
    pub fn format(table: &dyn RandomAccessFile) -> Result<(), Error> {
        let block_count = table.len() / 8 - 1;
        set_head(table, Some(0))?;
        set_node(
            table,
            0,
            Node {
                size: block_count,
                prev: None,
                next: None,
            },
        )
    }

    pub fn new(
        table: Rc<dyn RandomAccessFile>,
        data: Rc<dyn RandomAccessFile>,
        block_len: usize,
    ) -> Result<Rc<Fat>, Error> {
        let table_len = table.len();
        let data_len = data.len();
        if table_len % 8 != 0 {
            return make_error(Error::SizeMismatch);
        }
        let block_count = table_len / 8 - 1;
        if data_len != block_count * block_len {
            return make_error(Error::SizeMismatch);
        }

        let mut free_blocks = 0;
        if let Some(head) = get_head(table.as_ref())? {
            iterate_fat_entry(table.as_ref(), head, |_node_start, node_size| {
                free_blocks += node_size;
            })?;
        }

        Ok(Rc::new(Fat {
            table,
            data,
            block_len,
            free_blocks: Cell::new(free_blocks),
        }))
    }

    pub fn free_blocks(&self) -> usize {
        self.free_blocks.get()
    }
}

/// A handle to a file in `Fat` that implements resizing, releasing, reading and writing.
pub struct FatFile {
    fat: Rc<Fat>,
    block_list: Vec<BlockMap>,
}
impl FatFile {
    /// Opens the file at the specific block index.
    pub fn open(fat: Rc<Fat>, first_block: usize) -> Result<FatFile, Error> {
        let mut block_list = Vec::new();

        iterate_fat_entry(fat.table.as_ref(), first_block, |node_start, node_size| {
            for i in 0..node_size {
                block_list.push(BlockMap {
                    block_index: i + node_start,
                    node_start_index: node_start,
                });
            }
        })?;

        Ok(FatFile { fat, block_list })
    }

    /// Allocates a new file in `Fat` and returns its handle and block index.
    pub fn create(fat: Rc<Fat>, block_count: usize) -> Result<(FatFile, usize), Error> {
        if block_count == 0 {
            return make_error(Error::InvalidValue);
        }
        let free_blocks = fat.free_blocks.get();
        if free_blocks < block_count {
            return make_error(Error::NoSpace);
        }
        fat.free_blocks.set(free_blocks - block_count);

        let block_list = allocate(fat.table.as_ref(), block_count)?;
        let first = block_list[0].block_index;
        Ok((FatFile { fat, block_list }, first))
    }

    /// Releases the space this file holds.
    pub fn delete(self) -> Result<(), Error> {
        free(self.fat.table.as_ref(), &self.block_list)?;
        self.fat
            .free_blocks
            .set(self.fat.free_blocks.get() + self.block_list.len());
        Ok(())
    }

    /// Allocates more blocks for the file or releases some blocks.
    pub fn resize(&mut self, block_count: usize) -> Result<(), Error> {
        if block_count == 0 {
            return make_error(Error::InvalidValue);
        }
        if block_count == self.block_list.len() {
            return Ok(());
        }

        let table = self.fat.table.as_ref();

        let free_blocks = self.fat.free_blocks.get();

        if block_count > self.block_list.len() {
            let delta = block_count - self.block_list.len();
            if free_blocks < delta {
                return make_error(Error::NoSpace);
            }

            let mut block_list = allocate(table, delta)?;

            let tail_index = self.block_list.last().unwrap().node_start_index;
            let head_index = block_list[0].block_index;

            let mut tail = get_node(table, tail_index)?;
            tail.next = Some(head_index);
            set_node(table, tail_index, tail)?;

            let mut head = get_node(table, head_index)?;
            head.prev = Some(tail_index);
            set_node(table, head_index, head)?;

            self.block_list.append(&mut block_list);

            self.fat.free_blocks.set(free_blocks - delta);
        } else {
            let delta = self.block_list.len() - block_count;
            let head = &self.block_list[block_count];
            let head_index = head.block_index;
            if head_index == head.node_start_index {
                // we split the list right on node boundary
                let tail_index = self.block_list[block_count - 1].node_start_index;
                let mut tail = get_node(table, tail_index)?;
                tail.next = None;
                set_node(table, tail_index, tail)?;

                let mut head = get_node(table, head_index)?;
                head.prev = None;
                set_node(table, head_index, head)?;
            } else {
                // we need to split a node
                let tail_index = head.node_start_index;
                // modify the second half of the node in the list to form its own node
                for i in block_count..self.block_list.len() {
                    if self.block_list[i].node_start_index == tail_index {
                        self.block_list[i].node_start_index = head_index;
                    } else {
                        break;
                    }
                }

                // disconnect the first half
                let mut tail = get_node(table, tail_index)?;
                let tail_size = tail.size;
                tail.size = head_index - tail_index;
                let next = tail.next;
                tail.next = None;
                set_node(table, tail_index, tail)?;

                // create the second half in the table
                set_node(
                    table,
                    head_index,
                    Node {
                        prev: None,
                        next,
                        size: tail_size - (head_index - tail_index),
                    },
                )?;

                // also update the prev pointer of the node after the splitting node
                if let Some(next_index) = next {
                    let mut next = get_node(table, next_index)?;
                    if next.prev != Some(tail_index) {
                        error!("Inconsistent prev pointer detected while resizing");
                        return make_error(Error::BrokenFat);
                    }
                    next.prev = Some(head_index);
                    set_node(table, next_index, next)?;
                }
            }

            free(table, &self.block_list[block_count..])?;
            self.block_list.truncate(block_count);

            self.fat.free_blocks.set(free_blocks + delta);
        }

        Ok(())
    }
}

impl RandomAccessFile for FatFile {
    fn read(&self, pos: usize, buf: &mut [u8]) -> Result<(), Error> {
        let end = pos + buf.len();
        if end > self.len() {
            return make_error(Error::OutOfBound);
        }

        // block index range the operation covers
        let begin_block = pos / self.fat.block_len;
        let end_block = divide_up(end, self.fat.block_len);

        for i in begin_block..end_block {
            let data_begin_as_block = i * self.fat.block_len;
            let data_end_as_block = (i + 1) * self.fat.block_len;
            let data_begin = std::cmp::max(data_begin_as_block, pos);
            let data_end = std::cmp::min(data_end_as_block, end);
            let block_index = self.block_list[i].block_index;
            self.fat.data.read(
                block_index * self.fat.block_len + data_begin - data_begin_as_block,
                &mut buf[data_begin - pos..data_end - pos],
            )?;
        }
        Ok(())
    }
    fn write(&self, pos: usize, buf: &[u8]) -> Result<(), Error> {
        let end = pos + buf.len();
        if end > self.len() {
            return make_error(Error::OutOfBound);
        }

        // block index range the operation covers
        let begin_block = pos / self.fat.block_len;
        let end_block = divide_up(end, self.fat.block_len);

        for i in begin_block..end_block {
            let data_begin_as_block = i * self.fat.block_len;
            let data_end_as_block = (i + 1) * self.fat.block_len;
            let data_begin = std::cmp::max(data_begin_as_block, pos);
            let data_end = std::cmp::min(data_end_as_block, end);
            let block_index = self.block_list[i].block_index;
            self.fat.data.write(
                block_index * self.fat.block_len + data_begin - data_begin_as_block,
                &buf[data_begin - pos..data_end - pos],
            )?;
        }
        Ok(())
    }
    fn len(&self) -> usize {
        self.block_list.len() * self.fat.block_len
    }
    fn commit(&self) -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::fat::*;
    use crate::memory_file::MemoryFile;
    use std::rc::Rc;

    #[test]
    fn struct_size() {
        assert_eq!(Entry::BYTE_LEN, 8);
    }

    #[test]
    fn fuzz() {
        use rand::distributions::Standard;
        use rand::prelude::*;

        let mut rng = rand::thread_rng();
        for _ in 0..100 {
            let block_len = rng.gen_range(1, 10);
            let block_count = rng.gen_range(1, 100);

            let table = Rc::new(MemoryFile::new(vec![0; 8 * (block_count + 1)]));
            let data = Rc::new(MemoryFile::new(vec![0; block_count * block_len]));
            Fat::format(table.as_ref()).unwrap();
            let fat = Fat::new(table, data, block_len).unwrap();

            let mut free_block_count = block_count;

            struct File {
                image: Vec<u8>,
                fat_file: FatFile,
                start_block: usize,
            }

            let mut files = vec![];

            for _ in 0..10000 {
                let operation = rng.gen_range(1, 20);
                if operation < 2 {
                    // create file
                    let file_block_count = rng.gen_range(1, block_count / 2 + 2);
                    match FatFile::create(fat.clone(), file_block_count) {
                        Err(Error::NoSpace) => assert!(file_block_count > free_block_count),
                        Ok((fat_file, start_block)) => {
                            assert!(file_block_count <= free_block_count);
                            free_block_count -= file_block_count;
                            let image: Vec<u8> = rng
                                .sample_iter(&Standard)
                                .take(file_block_count * block_len)
                                .collect();
                            fat_file.write(0, &image).unwrap();
                            files.push(File {
                                image,
                                fat_file,
                                start_block,
                            });
                        }
                        _ => unreachable!(),
                    }
                } else if operation < 4 {
                    // open file
                    if files.is_empty() {
                        continue;
                    }

                    let file_index = rng.gen_range(0, files.len());
                    files[file_index].fat_file =
                        FatFile::open(fat.clone(), files[file_index].start_block).unwrap();
                } else if operation < 10 {
                    // read/write
                    if files.is_empty() {
                        continue;
                    }
                    let file_index = rng.gen_range(0, files.len());
                    let file = &mut files[file_index];
                    let len = file.image.len();
                    let pos = rng.gen_range(0, len);
                    let data_len = rng.gen_range(1, len - pos + 1);
                    if operation < 7 {
                        let a: Vec<u8> = rng.sample_iter(&Standard).take(data_len).collect();
                        file.fat_file.write(pos, &a).unwrap();
                        file.image[pos..pos + data_len].copy_from_slice(&a);
                    } else {
                        let mut a = vec![0; data_len];
                        file.fat_file.read(pos, &mut a).unwrap();
                        assert_eq!(a, &file.image[pos..pos + data_len]);
                    }
                } else if operation < 14 {
                    // delete
                    if files.is_empty() {
                        continue;
                    }
                    let file_index = rng.gen_range(0, files.len());
                    let file = files.remove(file_index);
                    free_block_count += file.image.len() / block_len;
                    file.fat_file.delete().unwrap();
                } else {
                    // resize
                    if files.is_empty() {
                        continue;
                    }
                    let file_index = rng.gen_range(0, files.len());
                    let file = &mut files[file_index];
                    let file_block_count = file.image.len() / block_len;
                    let new_block_count = rng.gen_range(1, file_block_count * 2 + 1);

                    if new_block_count > file_block_count {
                        let delta = new_block_count - file_block_count;
                        match file.fat_file.resize(new_block_count) {
                            Err(Error::NoSpace) => assert!(delta > free_block_count),
                            Ok(()) => {
                                let mut a: Vec<u8> =
                                    rng.sample_iter(&Standard).take(delta * block_len).collect();
                                file.fat_file
                                    .write(file_block_count * block_len, &a)
                                    .unwrap();
                                file.image.append(&mut a);
                                free_block_count -= delta;
                            }
                            _ => unreachable!(),
                        }
                    } else {
                        let delta = file_block_count - new_block_count;
                        file.fat_file.resize(new_block_count).unwrap();
                        file.image.truncate(new_block_count * block_len);
                        free_block_count += delta;
                    }
                }
            }
        }
    }
}
