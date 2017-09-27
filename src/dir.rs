use std::ascii::AsciiExt;
use std::fmt;
use std::io::prelude::*;
use std::io;
use std::io::{Cursor, ErrorKind, SeekFrom};
use std::str;
use byteorder::{LittleEndian, ReadBytesExt};
use chrono::{DateTime, Date, TimeZone, Local};

use fs::{FatSharedStateRef, FatSlice};
use file::FatFile;

pub(crate) enum FatDirReader<'a, 'b: 'a> {
    File(FatFile<'a, 'b>),
    Root(FatSlice<'a, 'b>),
}

impl <'a, 'b> Read for FatDirReader<'a, 'b> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            &mut FatDirReader::File(ref mut file) => file.read(buf),
            &mut FatDirReader::Root(ref mut raw) => raw.read(buf),
        }
    }
}

impl <'a, 'b> Seek for FatDirReader<'a, 'b> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match self {
            &mut FatDirReader::File(ref mut file) => file.seek(pos),
            &mut FatDirReader::Root(ref mut raw) => raw.seek(pos),
        }
    }
}



bitflags! {
    #[derive(Default)]
    pub struct FatFileAttributes: u8 {
        const READ_ONLY  = 0x01;
        const HIDDEN     = 0x02;
        const SYSTEM     = 0x04;
        const VOLUME_ID  = 0x08;
        const DIRECTORY  = 0x10;
        const ARCHIVE    = 0x20;
        const LFN        = Self::READ_ONLY.bits | Self::HIDDEN.bits
                         | Self::SYSTEM.bits | Self::VOLUME_ID.bits;
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
struct FatDirFileEntryData {
    name: [u8; 11],
    attrs: FatFileAttributes,
    reserved_0: u8,
    create_time_0: u8,
    create_time_1: u16,
    create_date: u16,
    access_date: u16,
    first_cluster_hi: u16,
    modify_time: u16,
    modify_date: u16,
    first_cluster_lo: u16,
    size: u32,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
struct FatDirLfnEntryData {
    order: u8,
    name_0: [u16; 5],
    attrs: FatFileAttributes,
    entry_type: u8,
    checksum: u8,
    name_1: [u16; 6],
    reserved_0: u16,
    name_2: [u16; 2],
}

#[derive(Clone, Debug)]
enum FatDirEntryData {
    File(FatDirFileEntryData),
    Lfn(FatDirLfnEntryData),
}

#[derive(Clone)]
pub struct FatDirEntry<'a, 'b: 'a> {
    data: FatDirFileEntryData,
    lfn: Vec<u16>,
    state: FatSharedStateRef<'a, 'b>,
}

impl <'a, 'b> FatDirEntry<'a, 'b> {
    pub fn short_file_name(&self) -> String {
        let name = str::from_utf8(&self.data.name[0..8]).unwrap().trim_right();
        let ext = str::from_utf8(&self.data.name[8..11]).unwrap().trim_right();
        if ext == "" { name.to_string() } else { format!("{}.{}", name, ext) }
    }
    
    pub fn file_name(&self) -> String {
        if self.lfn.len() > 0 {
            String::from_utf16(&self.lfn).unwrap()
        } else {
            self.short_file_name()
        }
    }
    
    pub fn attributes(&self) -> FatFileAttributes {
        self.data.attrs
    }
    
    pub fn is_dir(&self) -> bool {
        self.data.attrs.contains(FatFileAttributes::DIRECTORY)
    }
    
    pub fn is_file(&self) -> bool {
        !self.is_dir()
    }
    
    pub(crate) fn first_cluster(&self) -> u32 {
        ((self.data.first_cluster_hi as u32) << 16) | self.data.first_cluster_lo as u32
    }
    
    pub fn to_file(&self) -> FatFile<'a, 'b> {
        if self.is_dir() {
            panic!("This is a directory");
        }
        FatFile::new(self.first_cluster(), Some(self.data.size), self.state)
    }
    
    pub fn to_dir(&self) -> FatDir<'a, 'b> {
        if !self.is_dir() {
            panic!("This is a file");
        }
        let file = FatFile::new(self.first_cluster(), None, self.state);
        FatDir::new(FatDirReader::File(file), self.state)
    }
    
    pub fn len(&self) -> u64 {
        self.data.size as u64
    }
    
    pub fn created(&self) -> DateTime<Local> {
        Self::convert_date_time(self.data.create_date, self.data.create_time_1)
    }
    
    pub fn accessed(&self) -> Date<Local> {
        Self::convert_date(self.data.access_date)
    }
    
    pub fn modified(&self) -> DateTime<Local> {
        Self::convert_date_time(self.data.modify_date, self.data.modify_time)
    }
    
    fn convert_date(dos_date: u16) -> Date<Local> {
        let (year, month, day) = ((dos_date >> 9) + 1980, (dos_date >> 5) & 0xF, dos_date & 0x1F);
        Local.ymd(year as i32, month as u32, day as u32)
    }
    
    fn convert_date_time(dos_date: u16, dos_time: u16) -> DateTime<Local> {
        let (hour, min, sec) = (dos_time >> 11, (dos_time >> 5) & 0x3F, (dos_time & 0x1F) * 2);
        FatDirEntry::convert_date(dos_date).and_hms(hour as u32, min as u32, sec as u32)
    }
}

impl <'a, 'b> fmt::Debug for FatDirEntry<'a, 'b> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        self.data.fmt(f)
    }
}

pub struct FatDir<'a, 'b: 'a> {
    rdr: FatDirReader<'a, 'b>,
    state: FatSharedStateRef<'a, 'b>,
}

impl <'a, 'b> FatDir<'a, 'b> {
    
    pub(crate) fn new(rdr: FatDirReader<'a, 'b>, state: FatSharedStateRef<'a, 'b>) -> FatDir<'a, 'b> {
        FatDir { rdr, state }
    }
    
    pub fn list(&mut self) -> io::Result<Vec<FatDirEntry<'a, 'b>>> {
        self.rewind();
        Ok(self.map(|x| x.unwrap()).collect())
    }
    
    pub fn rewind(&mut self) {
        self.rdr.seek(SeekFrom::Start(0)).unwrap();
    }
    
    fn read_dir_entry_data(&mut self) -> io::Result<FatDirEntryData> {
        let mut name = [0; 11];
        self.rdr.read(&mut name)?;
        let attrs = FatFileAttributes::from_bits(self.rdr.read_u8()?).expect("invalid attributes");
        if attrs == FatFileAttributes::LFN {
            let mut data = FatDirLfnEntryData {
                attrs, ..Default::default()
            };
            let mut cur = Cursor::new(&name);
            data.order = cur.read_u8()?;
            cur.read_u16_into::<LittleEndian>(&mut data.name_0)?;
            data.entry_type = self.rdr.read_u8()?;
            data.checksum = self.rdr.read_u8()?;
            self.rdr.read_u16_into::<LittleEndian>(&mut data.name_1)?;
            data.reserved_0 = self.rdr.read_u16::<LittleEndian>()?;
            self.rdr.read_u16_into::<LittleEndian>(&mut data.name_2)?;
            Ok(FatDirEntryData::Lfn(data))
        } else {
            let data = FatDirFileEntryData {
                name,
                attrs,
                reserved_0:       self.rdr.read_u8()?,
                create_time_0:    self.rdr.read_u8()?,
                create_time_1:    self.rdr.read_u16::<LittleEndian>()?,
                create_date:      self.rdr.read_u16::<LittleEndian>()?,
                access_date:      self.rdr.read_u16::<LittleEndian>()?,
                first_cluster_hi: self.rdr.read_u16::<LittleEndian>()?,
                modify_time:      self.rdr.read_u16::<LittleEndian>()?,
                modify_date:      self.rdr.read_u16::<LittleEndian>()?,
                first_cluster_lo: self.rdr.read_u16::<LittleEndian>()?,
                size:             self.rdr.read_u32::<LittleEndian>()?,
            };
            Ok(FatDirEntryData::File(data))
        }
    }
    
    fn split_path<'c>(path: &'c str) -> (&'c str, Option<&'c str>) {
        let mut path_split = path.trim_matches('/').splitn(2, "/");
        let comp = path_split.next().unwrap();
        let rest_opt = path_split.next();
        (comp, rest_opt)
    }
    
    fn find_entry(&mut self, name: &str) -> io::Result<FatDirEntry<'a, 'b>> {
        let entries: Vec<FatDirEntry<'a, 'b>> = self.list()?;
        for e in entries {
            if e.file_name().eq_ignore_ascii_case(name) {
                return Ok(e);
            }
        }
        Err(io::Error::new(ErrorKind::NotFound, "file not found"))
    }
    
    pub fn open_dir(&mut self, path: &str) -> io::Result<FatDir<'a, 'b>> {
        let (name, rest_opt) = Self::split_path(path);
        let e = self.find_entry(name)?;
        match rest_opt {
            Some(rest) => e.to_dir().open_dir(rest),
            None => Ok(e.to_dir())
        }
    }
    
    pub fn open_file(&mut self, path: &str) -> io::Result<FatFile<'a, 'b>> {
        let (name, rest_opt) = Self::split_path(path);
        let e = self.find_entry(name)?;
        match rest_opt {
            Some(rest) => e.to_dir().open_file(rest),
            None => Ok(e.to_file())
        }
    }
}

impl <'a, 'b> Iterator for FatDir<'a, 'b> {
    type Item = io::Result<FatDirEntry<'a, 'b>>;

    fn next(&mut self) -> Option<io::Result<FatDirEntry<'a, 'b>>> {
        let mut lfn_buf = Vec::<u16>::new();
        loop {
            let res = self.read_dir_entry_data();
            let data = match res {
                Ok(data) => data,
                Err(err) => return Some(Err(err)),
            };
            match data {
                FatDirEntryData::File(data) => {
                    // Check if this is end of dif
                    if data.name[0] == 0 {
                        return None;
                    }
                    // Check if this is deleted or volume ID entry
                    if data.name[0] == 0xE5 || data.attrs.contains(FatFileAttributes::VOLUME_ID) {
                        lfn_buf.clear();
                        continue;
                    }
                    // Truncate 0 and 0xFFFF characters from LFN buffer
                    let mut lfn_len = lfn_buf.len();
                    loop {
                        if lfn_len == 0 {
                            break;
                        }
                        match lfn_buf[lfn_len-1] {
                            0xFFFF | 0 => lfn_len -= 1,
                            _ => break,
                        }
                    }
                    lfn_buf.truncate(lfn_len);
                    return Some(Ok(FatDirEntry {
                        data,
                        lfn: lfn_buf,
                        state: self.state.clone(),
                    }));
                },
                FatDirEntryData::Lfn(data) => {
                    // Check if this is deleted entry
                    if data.order == 0xE5 {
                        lfn_buf.clear();
                        continue;
                    }
                    const LFN_PART_LEN: usize = 13;
                    let index = (data.order & 0x1F) - 1;
                    let pos = LFN_PART_LEN * index as usize;
                    // resize LFN buffer to have enough space for entire name
                    if lfn_buf.len() < pos + LFN_PART_LEN {
                       lfn_buf.resize(pos + LFN_PART_LEN, 0);
                    }
                    // copy name parts into LFN buffer
                    lfn_buf[pos+0..pos+5].clone_from_slice(&data.name_0);
                    lfn_buf[pos+5..pos+11].clone_from_slice(&data.name_1);
                    lfn_buf[pos+11..pos+13].clone_from_slice(&data.name_2);
                }
            };
        }
    }
}
