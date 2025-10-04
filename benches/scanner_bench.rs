use criterion::{black_box, criterion_group, criterion_main, Criterion};
use dirstat::scanner::{Scanner, ScanConfig};
use std::path::Path;
use tempfile::TempDir;

fn create_test_directory() -> TempDir {
    let temp_dir = TempDir::new().unwrap();

    // Create some test files and directories
    std::fs::create_dir_all(temp_dir.path().join("deep/directory/structure")).unwrap();
    std::fs::write(temp_dir.path().join("file1.txt"), "content").unwrap();
    std::fs::write(temp_dir.path().join("file2.txt"), "more content").unwrap();
    std::fs::write(temp_dir.path().join("deep/file3.txt"), "deep content").unwrap();

    temp_dir
}

fn bench_scanner_small(c: &mut Criterion) {
    let temp_dir = create_test_directory();

    c.bench_function("scan_small_directory", |b| {
        b.iter(|| {
            let scanner = Scanner::new(ScanConfig::default());
            let _ = scanner.scan(black_box(temp_dir.path()));
        });
    });
}

fn bench_scanner_large(c: &mut Criterion) {
    // This would test with a larger directory structure
    // For now, we'll use the small one
    let temp_dir = create_test_directory();

    c.bench_function("scan_large_directory", |b| {
        b.iter(|| {
            let scanner = Scanner::new(ScanConfig::default());
            let _ = scanner.scan(black_box(temp_dir.path()));
        });
    });
}

criterion_group!(benches, bench_scanner_small, bench_scanner_large);
criterion_main!(benches);
