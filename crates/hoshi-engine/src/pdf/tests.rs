use super::install::{build_for, install, verify};
use super::{Document, Match, PdfFailure, drawn_width};
use flate2::Compression;
use flate2::write::GzEncoder;
use std::path::{Path, PathBuf};

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hoshi-pdf-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    for (path, content) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, path, *content).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

/// A PDF of `pages` pages, each saying `Page <n>` and, on the last, `Total sales`.
fn pdf(path: &Path, pages: usize) {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {pages} >>",
            (0..pages)
                .map(|page| format!("{} 0 R", 4 + page * 2))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    for page in 0..pages {
        let mut text = format!("BT /F1 18 Tf 72 720 Td (Page {}) Tj ET", page + 1);
        if page + 1 == pages {
            text.push_str(" BT /F1 12 Tf 72 680 Td (Total sales: 1.234.567) Tj ET");
        }
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
            5 + page * 2
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{text}\nendstream",
            text.len()
        ));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{object}\nendobj\n", index + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .bytes(),
    );
    std::fs::write(path, out).unwrap();
}

fn search(document: &Document, query: &str) -> Vec<Match> {
    (0..document.pages().len())
        .flat_map(|page| document.search_page(page, query))
        .collect()
}

#[test]
fn a_page_too_large_to_draw_is_drawn_narrower() {
    assert_eq!(drawn_width(1200, 0.75), 1200);
    let long = drawn_width(3000, 0.1);
    assert!(long as f32 / 0.1 <= 8192., "{long}");
    let huge = drawn_width(20_000, 1.);
    assert!((huge * huge) as f32 <= 16_000_000., "{huge}");
    assert!(huge > 3900);
    assert_eq!(drawn_width(0, 1.), 1);
}

#[test]
fn every_system_hoshi_ships_for_has_a_pinned_build() {
    for os in ["windows", "linux", "macos"] {
        for arch in ["x86_64", "aarch64"] {
            let build = build_for(os, arch).unwrap();
            assert_eq!(build.sha256.len(), 64, "{os} {arch}");
        }
    }
    assert_eq!(
        build_for("windows", "x86_64").unwrap().entry,
        "bin/pdfium.dll"
    );
    assert_eq!(build_for("freebsd", "x86_64"), None);
}

#[test]
fn an_archive_that_is_not_the_pinned_build_is_refused() {
    let build = build_for("linux", "x86_64").unwrap();

    assert_eq!(verify(b"not pdfium", &build), Err(PdfFailure::Download));
}

#[test]
fn only_the_library_is_taken_from_the_archive() {
    let build = build_for("windows", "x86_64").unwrap();
    let dir = folder("install");
    let archive = archive(&[
        ("include/fpdfview.h", b"header"),
        ("bin/pdfium.dll", b"library"),
        ("LICENSE", b"license"),
    ]);

    install(&archive, &build, &dir).unwrap();

    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["pdfium.dll"]);
    assert_eq!(std::fs::read(dir.join("pdfium.dll")).unwrap(), b"library");
}

#[test]
fn an_archive_without_the_library_installs_nothing() {
    let build = build_for("linux", "x86_64").unwrap();
    let dir = folder("missing");

    assert!(install(&archive(&[("LICENSE", b"license")]), &build, &dir).is_err());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

/// Fetches the real library: `cargo test -p hoshi-engine pdf -- --ignored`.
#[test]
#[ignore]
fn a_pdf_opens_draws_its_pages_and_finds_its_text() {
    let dir = folder("live");
    let path = dir.join("report.pdf");
    pdf(&path, 3);
    let broken = dir.join("broken.pdf");
    std::fs::write(&broken, "not a pdf").unwrap();

    let document = Document::open(&dir, &path).unwrap();

    assert_eq!(document.pages().len(), 3);
    assert_eq!(document.pages()[0].width, 612.);
    let page = document.render(0, 306).unwrap();
    assert_eq!((page.width, page.height), (306, 396));
    assert_eq!(page.bgra.len(), 306 * 396 * 4);
    assert!(page.bgra.chunks(4).any(|pixel| pixel[0] < 128));
    let found = search(&document, "total SALES");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].page, 2);
    let area = found[0].areas[0];
    assert!(area.left > 0.1 && area.left < 0.15, "{area:?}");
    assert!(area.top > 0.1 && area.top < 0.16, "{area:?}");
    assert_eq!(search(&document, "page").len(), 3);
    assert!(search(&document, "  ").is_empty());
    assert_eq!(Document::open(&dir, &broken).err(), Some(PdfFailure::Read));
    assert!(dir.join("tools/pdfium/8125").is_dir());
}
