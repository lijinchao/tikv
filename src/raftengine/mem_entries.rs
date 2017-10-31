
use std::cmp;
use std::collections::VecDeque;

use kvproto::eraftpb::Entry;

const SHRINK_CACHE_CAPACITY: usize = 64;

pub struct FileIndex {
    pub file_num: u64,

    // last entry index in this file.
    pub last_index: u64,
}

impl FileIndex {
    pub fn new(file_num: u64, last_index: u64) -> FileIndex {
        FileIndex {
            file_num: file_num,
            last_index: last_index,
        }
    }
}

pub struct MemEntries {
    pub region_id: u64,
    pub entry_cache: VecDeque<Entry>,
    pub file_index: VecDeque<FileIndex>,
}

impl MemEntries {
    pub fn new(region_id: u64) -> MemEntries {
        MemEntries {
            region_id: region_id,
            entry_cache: VecDeque::with_capacity(SHRINK_CACHE_CAPACITY),
            file_index: VecDeque::new(),
        }
    }

    pub fn append(&mut self, entries: &[Entry], file_num: u64) {
        if entries.is_empty() {
            return;
        }
        if let Some(cache_last_index) = self.entry_cache.back().map(|e| e.get_index()) {
            let first_index = entries[0].get_index();

            // Unlikely to happen
            if cache_last_index >= first_index {
                if self.entry_cache.front().unwrap().get_index() >= first_index {
                    self.entry_cache.clear();
                    self.file_index.clear();
                } else {
                    let left =
                        self.entry_cache.len() - (cache_last_index - first_index + 1) as usize;
                    self.entry_cache.truncate(left);

                    let cache_last_index = self.entry_cache.back().unwrap().get_index();
                    self.truncate_file_index(cache_last_index);
                }
            } else if cache_last_index + 1 < first_index {
                panic!(
                    "entry cache of region {} contains unexpected hole: {} < {}",
                    self.region_id,
                    cache_last_index,
                    first_index
                );
            }
        }

        // Add entries
        for e in entries {
            self.entry_cache.push_back(e.to_owned());
        }

        // Update file index.
        let cache_last_index = self.entry_cache.back().unwrap().get_index();
        if !self.file_index.is_empty() && self.file_index.back().unwrap().file_num == file_num {
            self.file_index.back_mut().unwrap().last_index = cache_last_index;
        } else {
            self.file_index
                .push_back(FileIndex::new(file_num, cache_last_index));
        }
    }

    pub fn compact_to(&mut self, idx: u64) {
        let cache_first_idx = match self.entry_cache.front() {
            None => return,
            Some(e) => e.get_index(),
        };

        if cache_first_idx > idx {
            return;
        }
        let cache_last_idx = self.entry_cache.back().unwrap().get_index();
        self.entry_cache
            .drain(..(cmp::min(cache_last_idx, idx) - cache_first_idx) as usize);

        if self.entry_cache.len() < SHRINK_CACHE_CAPACITY &&
            self.entry_cache.capacity() > SHRINK_CACHE_CAPACITY
        {
            self.entry_cache.shrink_to_fit();
        }

        let first_index = match self.entry_cache.front() {
            Some(e) => e.get_index(),
            None => {
                self.file_index.clear();
                return;
            }
        };

        let pos = self.file_index_pos(first_index).unwrap_or_else(|| {
            panic!("Can't find file index for entry index: {}", first_index)
        });
        if pos > 0 {
            self.file_index.drain(..pos);
        }
    }

    pub fn fetch_all(&self, vec: &mut Vec<Entry>) {
        for e in &self.entry_cache {
            vec.push(e.clone());
        }
    }

    pub fn min_file_num(&self) -> Option<u64> {
        self.file_index.front().map(|index| index.file_num)
    }

    pub fn max_file_num(&self) -> Option<u64> {
        self.file_index.back().map(|index| index.file_num)
    }

    fn file_index_pos(&self, entry_index: u64) -> Option<usize> {
        for (pos, index) in self.file_index.iter().enumerate() {
            if index.last_index >= entry_index {
                return Some(pos);
            }
        }
        None
    }

    fn truncate_file_index(&mut self, cache_last_index: u64) {
        let pos = self.file_index_pos(cache_last_index).unwrap_or_else(|| {
            panic!(
                "Can't find associated file index for entry index: {}",
                cache_last_index
            )
        });
        self.file_index[pos].last_index = cache_last_index;
        self.file_index.drain(pos + 1..);
    }
}
