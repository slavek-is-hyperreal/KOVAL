use std::fs::{self, OpenOptions};
use std::io::{Read, Write, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::time::Instant;
use std::alloc::{alloc, dealloc, Layout};
use schema::StorageProfile;

fn detect_io_uring_internal<F1, F2>(sys_path_exists: F1, read_proc_file: F2) -> bool
where
    F1: Fn(&str) -> bool,
    F2: Fn(&str) -> Option<String>,
{
    if !sys_path_exists("/sys/module/io_uring") {
        return false;
    }
    match read_proc_file("/proc/sys/kernel/io_uring_disabled") {
        Some(content) => content.trim() == "0",
        None => true,
    }
}

pub fn collect() -> StorageProfile {
    let io_uring = detect_io_uring_internal(
        |path| fs::metadata(path).is_ok(),
        |path| fs::read_to_string(path).ok(),
    );
    let o_direct = test_o_direct_support();
    let (read_speed_mbs, write_speed_mbs) = run_disk_benchmark(o_direct);

    StorageProfile {
        io_uring,
        o_direct,
        read_speed_mbs,
        write_speed_mbs,
    }
}

struct AlignedBuffer {
    ptr: *mut u8,
    layout: Layout,
    size: usize,
}

impl AlignedBuffer {
    fn new(size: usize, align: usize) -> Self {
        let layout = Layout::from_size_align(size, align).unwrap();
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            panic!("Allocation failed");
        }
        Self { ptr, layout, size }
    }

    fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.size) }
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.size) }
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.ptr, self.layout);
        }
    }
}

fn test_o_direct_support() -> bool {
    let temp_path = "/tmp/koval_direct_test.tmp";
    // libc::O_DIRECT is 0x4000 on Linux x86_64 and arm64
    let result = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(0x4000)
        .open(temp_path);

    let supported = result.is_ok();
    let _ = fs::remove_file(temp_path);
    supported
}

fn run_disk_benchmark(use_o_direct: bool) -> (f64, f64) {
    let temp_path = "/tmp/koval_disk_bench.tmp";
    let data_size = 5 * 1024 * 1024; // 5MB

    // Create write buffer with 512-byte alignment
    let mut write_buf = AlignedBuffer::new(data_size, 512);
    write_buf.as_mut_slice().fill(0xAA);

    // Open options
    let mut options = OpenOptions::new();
    options.write(true).read(true).create(true).truncate(true);
    if use_o_direct {
        options.custom_flags(0x4000);
    }

    let mut file = match options.open(temp_path) {
        Ok(f) => f,
        Err(_) => return (0.0, 0.0),
    };

    // Write benchmark
    let start_write = Instant::now();
    if file.write_all(write_buf.as_slice()).is_err() {
        let _ = fs::remove_file(temp_path);
        return (0.0, 0.0);
    }
    // Flush to ensure data is written to disk
    if file.sync_all().is_err() {
        let _ = fs::remove_file(temp_path);
        return (0.0, 0.0);
    }
    let write_elapsed = start_write.elapsed().as_secs_f64();

    // Seek to beginning for reading
    if file.seek(SeekFrom::Start(0)).is_err() {
        let _ = fs::remove_file(temp_path);
        return (0.0, 0.0);
    }

    // Read benchmark with 512-byte alignment
    let mut read_buf = AlignedBuffer::new(data_size, 512);
    let start_read = Instant::now();
    if file.read_exact(read_buf.as_mut_slice()).is_err() {
        let _ = fs::remove_file(temp_path);
        return (0.0, 0.0);
    }
    let read_elapsed = start_read.elapsed().as_secs_f64();

    let _ = fs::remove_file(temp_path);

    let size_mb = (data_size as f64) / (1024.0 * 1024.0);
    let write_speed = if write_elapsed > 0.0 { size_mb / write_elapsed } else { 0.0 };
    let read_speed = if read_elapsed > 0.0 { size_mb / read_elapsed } else { 0.0 };

    (read_speed, write_speed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_io_uring_detection_disabled() {
        // Test 19: sysfs exists but disabled in proc (value "1" or "2")
        let sys_exists = |_: &str| true;
        let proc_val_1 = |_: &str| Some("1\n".to_string());
        let proc_val_2 = |_: &str| Some("2".to_string());

        assert!(!detect_io_uring_internal(sys_exists, proc_val_1));
        assert!(!detect_io_uring_internal(sys_exists, proc_val_2));
    }

    #[test]
    fn test_io_uring_detection_enabled() {
        // Test 20: sysfs exists, proc has "0" or is missing
        let sys_exists = |_: &str| true;
        let proc_val_0 = |_: &str| Some(" 0 \n".to_string());
        let proc_missing = |_: &str| None;

        assert!(detect_io_uring_internal(sys_exists, proc_val_0));
        assert!(detect_io_uring_internal(sys_exists, proc_missing));

        // Also if sysfs is missing, should always be false
        let sys_missing = |_: &str| false;
        assert!(!detect_io_uring_internal(sys_missing, proc_val_0));
        assert!(!detect_io_uring_internal(sys_missing, proc_missing));
    }
}
