//! #288: an AVC sample entry may describe the coded sensor area rather than the cropped picture.
//! Synthetic H.264 syntax follows ITU-T H.264 §7.3.2.1.1 / §7.3.5, without external fixtures.

use super::*;
use filmcraft_bitstream::{BitWriter, escape_rbsp};
use filmcraft_isobmff::{AvcConfig, Brand, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};

const ONE: i32 = 0x10000;
const IDENTITY: [i32; 9] = [ONE, 0, 0, 0, ONE, 0, 0, 0, 0x4000_0000];
const CLOCKWISE: [i32; 9] = [0, ONE, 0, -ONE, 0, 0, 48 * ONE, 0, 0x4000_0000];

fn nal(kind: u8, w: BitWriter) -> Vec<u8> {
    let mut out = vec![0x60 | kind];
    out.extend(escape_rbsp(&w.finish()));
    out
}

fn sps(width: u32, height: u32, chroma: u32, progressive: bool, crop: Option<(u32, u32, u32, u32)>) -> Vec<u8> {
    sps_with_id(width, height, chroma, progressive, crop, 0)
}

fn sps_with_id(width: u32, height: u32, chroma: u32, progressive: bool, crop: Option<(u32, u32, u32, u32)>, id: u32) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(100, 8); // High profile
    w.write_bits(0, 8); // constraints
    w.write_bits(40, 8); // level
    w.write_ue(id); // SPS id
    w.write_ue(chroma);
    if chroma == 3 {
        w.write_bit(false); // separate_colour_plane_flag
    }
    w.write_ue(0); // 8-bit luma
    w.write_ue(0); // 8-bit chroma
    w.write_bits(0, 2); // no transform bypass or scaling matrices
    w.write_ue(0); // log2_max_frame_num_minus4
    w.write_ue(2); // POC follows frame_num
    w.write_ue(1); // one reference
    w.write_bit(false); // no frame_num gaps
    w.write_ue(width / 16 - 1);
    w.write_ue(height / (if progressive { 16 } else { 32 }) - 1);
    w.write_bit(progressive);
    if !progressive {
        w.write_bit(false); // no adaptive frame/field macroblocks
    }
    w.write_bit(true); // direct_8x8_inference
    w.write_bit(crop.is_some());
    if let Some((left, right, top, bottom)) = crop {
        for offset in [left, right, top, bottom] {
            w.write_ue(offset);
        }
    }
    w.write_bit(false); // no VUI
    w.rbsp_trailing();
    nal(7, w)
}

fn pps() -> Vec<u8> {
    pps_with_ids(0, 0)
}

fn pps_with_ids(pps_id: u32, sps_id: u32) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(pps_id);
    w.write_ue(sps_id);
    w.write_bits(0, 2); // CAVLC, no bottom-field POC
    w.write_ue(0); // no slice groups
    w.write_ue(0); // one reference in each list
    w.write_ue(0);
    w.write_bits(0, 3); // unweighted prediction
    w.write_se(0); // QP and QS offsets
    w.write_se(0);
    w.write_se(0); // chroma QP offset
    w.write_bits(0b100, 3); // deblocking control, unconstrained intra, no redundant pictures
    w.rbsp_trailing();
    nal(8, w)
}

/// Lossless I_PCM picture: a luma ramp identifies the pixels retained by a crop / rotation.
fn picture() -> Vec<u8> {
    picture_with_pps(0)
}

fn picture_with_pps(pps_id: u32) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0); // first macroblock
    w.write_ue(7); // I slice
    w.write_ue(pps_id); // PPS id
    w.write_bits(0, 4); // frame_num
    w.write_ue(0); // IDR id
    w.write_bits(0, 2); // no_output_of_prior_pics, long_term_reference
    w.write_se(0); // slice_qp_delta
    w.write_ue(1); // disable deblocking
    for my in 0..3 {
        for mx in 0..4 {
            w.write_ue(25); // I_PCM
            w.align_zero();
            for y in 0..16 {
                for x in 0..16 {
                    w.write_bits(luma(mx * 16 + x, my * 16 + y) as u32, 8);
                }
            }
            for _ in 0..128 {
                w.write_bits(128, 8); // Cb, Cr
            }
        }
    }
    w.rbsp_trailing();
    nal(5, w)
}

fn luma(x: usize, y: usize) -> u8 {
    (20 + x + y * 2) as u8
}

fn mov(sps: Vec<u8>, coded_size: (u16, u16), matrix: [i32; 9], par: (u32, u32)) -> Arc<[u8]> {
    mov_sample(AvcConfig::new(vec![sps], vec![pps()], 4), length_prefixed(&[picture()]), coded_size, matrix, par)
}

fn length_prefixed(nals: &[Vec<u8>]) -> Vec<u8> {
    let mut sample = Vec::new();
    for nal in nals {
        sample.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        sample.extend_from_slice(nal);
    }
    sample
}

fn mov_sample(cfg: AvcConfig, sample: Vec<u8>, coded_size: (u16, u16), matrix: [i32; 9], par: (u32, u32)) -> Arc<[u8]> {
    let mut entry = SampleEntry::avc(cfg, coded_size.0, coded_size.1);
    entry.video.as_mut().expect("video").pixel_aspect = Some(par);
    let mut mux = Mp4Writer::new(std::io::Cursor::new(Vec::new()), WriterOptions::new(Brand::Mov)).expect("writer");
    let track = mux.add_track(TrackConfig::new(entry, 25)).expect("track");
    mux.write_sample(track, WriteSample { data: &sample, duration: 1, composition_offset: 0, is_sync: true }).expect("sample");
    let mut bytes = mux.finish().expect("finish").into_inner();
    let k = bytes.windows(4).position(|b| b == b"tkhd").expect("tkhd") + 4;
    let m = k + 4 + if bytes[k] == 1 { 32 } else { 20 } + 16;
    for (j, v) in matrix.into_iter().enumerate() {
        bytes[m + j * 4..m + j * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    bytes.into()
}

#[test]
fn phone_crop_metadata_precedes_display_rotation() {
    // Reported iPhone dimensions: 1920×1440 coded, 88 horizontal / 66 vertical pixels per edge.
    let phone = sps(1920, 1440, 1, true, Some((44, 44, 33, 33)));
    for (matrix, want, par) in [(IDENTITY, (1744, 1308), (2, 1)), (CLOCKWISE, (1308, 1744), (1, 2))] {
        let src = Mp4Source::open("phone.mov", mov(phone.clone(), (1920, 1440), matrix, (2, 1))).expect("open");
        let v = src.info().video.as_ref().expect("video");
        assert_eq!((v.width, v.height), want);
        assert_eq!(v.par, par);
    }
    // Crop units depend on chroma format and whether the SPS describes a frame or fields.
    for (chroma, progressive, want) in [(0, true, (1880, 1408)), (1, false, (1840, 1312)), (2, true, (1840, 1408)), (3, true, (1880, 1408))] {
        let src = Mp4Source::open("crop.mov", mov(sps(1920, 1440, chroma, progressive, Some((20, 20, 16, 16))), (1920, 1440), IDENTITY, (1, 1))).expect("open");
        let v = src.info().video.as_ref().expect("video");
        assert_eq!((v.width, v.height), want);
    }
}

#[test]
fn cropped_avc_metadata_matches_decoded_pixels() {
    let sps = sps(64, 48, 1, true, Some((4, 6, 3, 5))); // 44×32, starting at (8, 6)
    for (matrix, want, entry_size) in [(IDENTITY, (44, 32), (64, 48)), (IDENTITY, (44, 32), (44, 32)), (CLOCKWISE, (32, 44), (64, 48))] {
        // A container already reporting the visible size must not have the offsets subtracted twice.
        let src = Mp4Source::open("crop.mov", mov(sps.clone(), entry_size, matrix, (1, 1))).expect("open");
        let v = src.info().video.as_ref().expect("video");
        let frame = src.video_frame(FrameRequest::full(Tick::ZERO)).expect("decode");
        assert_eq!((v.width, v.height), want);
        assert_eq!((frame.width, frame.height), want);
        let expected: Vec<u8> = if matrix == IDENTITY {
            (6..38).flat_map(|y| (8..52).map(move |x| luma(x, y))).collect()
        } else {
            (8..52).flat_map(|x| (6..38).rev().map(move |y| luma(x, y))).collect()
        };
        assert_eq!(frame.luma8(), expected, "crop offsets must be applied before rotation");
    }
}

#[test]
fn first_picture_selects_its_sps_among_unused_parameter_sets() {
    let unused = sps(64, 48, 1, true, None);
    let active = sps_with_id(64, 48, 1, true, Some((4, 6, 3, 5)), 1);
    let cfg = AvcConfig::new(vec![unused, active], vec![pps(), pps_with_ids(3, 1)], 4);
    let sample = length_prefixed(&[picture_with_pps(3)]);
    let src = Mp4Source::open("multi-sps.mov", mov_sample(cfg, sample, (64, 48), CLOCKWISE, (1, 1))).expect("open");
    let v = src.info().video.as_ref().expect("video");
    let frame = src.video_frame(FrameRequest::full(Tick::ZERO)).expect("decode");
    assert_eq!((v.width, v.height), (32, 44));
    assert_eq!((v.width, v.height), (frame.width, frame.height));
    let expected: Vec<u8> = (8..52).flat_map(|x| (6..38).rev().map(move |y| luma(x, y))).collect();
    assert_eq!(frame.luma8(), expected);
}

#[test]
fn in_band_parameter_sets_define_the_first_picture_crop() {
    let cfg = AvcConfig::new(vec![sps(64, 48, 1, true, None)], vec![pps()], 4);
    let active = sps_with_id(64, 48, 1, true, Some((4, 6, 3, 5)), 1);
    let sample = length_prefixed(&[active, pps_with_ids(3, 1), picture_with_pps(3)]);
    let src = Mp4Source::open("in-band.mov", mov_sample(cfg, sample, (64, 48), IDENTITY, (1, 1))).expect("open");
    let v = src.info().video.as_ref().expect("video");
    let frame = src.video_frame(FrameRequest::full(Tick::ZERO)).expect("decode");
    assert_eq!((v.width, v.height), (44, 32));
    assert_eq!((v.width, v.height), (frame.width, frame.height));
}

#[test]
fn unreadable_sps_preserves_container_size_without_panicking() {
    for bad in [vec![0x67, 100], sps(64, 48, 1, true, Some((32, 0, 0, 0))), sps(64, 48, 1, true, Some((u32::MAX, 1, 0, 0)))] {
        let src = std::panic::catch_unwind(|| Mp4Source::open("bad.mov", mov(bad, (64, 48), IDENTITY, (1, 1)))).expect("no panic").expect("metadata");
        let v = src.info().video.as_ref().expect("video");
        assert_eq!((v.width, v.height), (64, 48));
    }
}

#[test]
fn cropped_rotated_avc_matches_ffmpeg() {
    let ff = filmcraft_testkit::require_ffmpeg!();
    for (tag, matrix) in [("upright", IDENTITY), ("rotated", CLOCKWISE)] {
        let bytes = mov(sps(64, 48, 1, true, Some((4, 6, 3, 5))), (64, 48), matrix, (1, 1));
        let path = filmcraft_testkit::fixtures_dir("codecs").join(format!("sps-crop-{tag}.mov"));
        std::fs::write(&path, &bytes).expect("fixture");
        let out = std::process::Command::new(&ff)
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args(["-map", "0:v:0", "-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
            .output()
            .expect("ffmpeg");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let src = Mp4Source::open("crop.mov", bytes).expect("open");
        let frame = src.video_frame(FrameRequest::full(Tick::ZERO)).expect("decode");
        let filmcraft_frame::PixelData::Yuv8 { planes, .. } = &frame.data else { panic!("YUV frame") };
        let decoded: Vec<u8> = planes.iter().flat_map(|p| p.iter().copied()).collect();
        assert_eq!(decoded, out.stdout, "ffmpeg crop/rotation oracle: {tag}");
    }
}
