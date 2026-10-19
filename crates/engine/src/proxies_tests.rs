//! Proxies end to end: create (background job) + attach + toggle gives the same composite modulo
//! resolution (scale-aware effects and Motion), attach checks, export ignores proxies, toggling
//! doesn't dirty the project, ingest copies on import.

use filmcraft_media::DemoScene;
use filmcraft_project::{ParamValue, find_effect};
use serde_json::json;

use crate::Session;
use crate::media_test_util::{frame_rgba, make_movie, psnr, session_with, tmp_dir};

const W: u32 = 320;
const H: u32 = 180;

/// Give V1's clip a Motion move and a blur, so proxies must render effects scale-aware.
fn add_effects(s: &mut Session) {
    let seq = s.state.active_sequence.unwrap();
    let mut p = (*s.project).clone();
    let q = p.sequence_mut(seq).unwrap();
    let it = &mut q.video_tracks[0].items[0];
    let m = it.effect_mut("motion").unwrap();
    m.params.get_mut("scale").unwrap().value = ParamValue::Float(70.0);
    m.params.get_mut("rotation").unwrap().value = ParamValue::Float(8.0);
    let mut blur = find_effect("gaussian_blur").unwrap().instance();
    if let Some(b) = blur.params.get_mut("blurriness") {
        b.value = ParamValue::Float(6.0);
    }
    it.effects.push(blur);
    s.project = std::sync::Arc::new(p);
}

#[cfg(not(target_arch = "wasm32"))]
mod pending_outputs {
    use super::*;
    use filmcraft_frame::{AudioBuffer, VideoFrame};
    use filmcraft_media::{FrameRequest, MediaInfo, MediaSource, SharedSource};
    use filmcraft_project::MediaRef;
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    /// Pause an actual decoded source on its first export frame, before the next queued take.
    struct PausedVideo {
        inner: SharedSource,
        entered: mpsc::Sender<()>,
        release: Mutex<Option<mpsc::Receiver<()>>>,
    }

    impl MediaSource for PausedVideo {
        fn info(&self) -> &MediaInfo {
            self.inner.info()
        }

        fn video_frame(&self, request: FrameRequest) -> filmcraft_media::Result<Arc<VideoFrame>> {
            let release = self.release.lock().unwrap().take();
            if let Some(release) = release {
                self.entered.send(()).map_err(|e| filmcraft_media::MediaError::Decode(e.to_string()))?;
                release.recv_timeout(Duration::from_secs(60)).map_err(|e| filmcraft_media::MediaError::Decode(e.to_string()))?;
            }
            self.inner.video_frame(request)
        }

        fn audio(&self, start: i64, frames: usize, rate: u32) -> filmcraft_media::Result<AudioBuffer> {
            self.inner.audio(start, frames, rate)
        }
    }

    fn drain(jobs: &[crate::Job]) -> bool {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if jobs.iter().all(|job| job.result.lock().unwrap_or_else(|e| e.into_inner()).is_some()) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::yield_now();
        }
    }

    struct Cleanup {
        root: PathBuf,
        release: Option<mpsc::Sender<()>>,
        jobs: Vec<crate::Job>,
    }

    impl Cleanup {
        fn release(&mut self) {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
        }
    }

    impl Drop for Cleanup {
        fn drop(&mut self) {
            // On an allocation assertion failure, cancel before waking the first producer so
            // it cannot advance to the still-unwritten take shared by the failing baseline.
            for job in &self.jobs {
                if job.result.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
                    job.progress.cancel.store(true, Ordering::Relaxed);
                }
            }
            self.release();
            if drain(&self.jobs) {
                let _ = std::fs::remove_dir_all(&self.root);
            } else {
                eprintln!("proxy fixture retained because a job has no terminal result: {}", self.root.display());
            }
        }
    }

    #[test]
    fn concurrent_proxy_jobs_reserve_unwritten_paths() {
        let root = tmp_dir("proxy-pending-output");
        let (release, released) = mpsc::channel();
        let mut cleanup = Cleanup { root: root.clone(), release: Some(release), jobs: Vec::new() };
        let blocker = root.join("blocker.mov");
        let paths = [root.join("Card A/A001.mov"), root.join("Card B/A001.mov")];
        make_movie(&blocker, DemoScene::Forest, 64, 64, 4);
        for (path, scene, width) in [(&paths[0], DemoScene::Aurora, 192), (&paths[1], DemoScene::Plasma, 128)] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            make_movie(path, scene, width, 96, 4);
        }
        let originals: Vec<_> = paths.iter().map(|path| std::fs::read(path).unwrap()).collect();
        assert_ne!(originals[0], originals[1], "distinct camera takes share only a basename");
        let (mut s, items, _) = session_with(&[&blocker, &paths[0], &paths[1]]);
        let source = s.media.full_res_source(&s.project, items[0], &*s.services).unwrap();
        let (entered, reached) = mpsc::channel();
        s.media.insert_file(items[0], &blocker.to_string_lossy(), Arc::new(PausedVideo { inner: source, entered, release: Mutex::new(Some(released)) }));
        let destination = root.join("Proxies");
        let first = s.execute(
            "media.createProxies",
            json!({"items": [items[0].0, items[1].0], "preset": "prores_proxy_half", "destination": destination.to_string_lossy()}),
        );
        cleanup.jobs = s.jobs.clone();
        let first = first.unwrap();
        reached.recv_timeout(Duration::from_secs(15)).expect("first job reached its real video decoder");
        let pending = first["outputs"][1]["path"].as_str().unwrap().to_owned();
        assert_eq!(first["outputs"][1]["item"], json!(items[1].0));
        assert!(!std::path::Path::new(&pending).exists(), "the next take is planned but has no output file yet");
        assert!(cleanup.jobs[0].result.lock().unwrap().is_none(), "first job is still blocked");

        let second =
            s.execute("media.createProxies", json!({"items": [items[2].0], "preset": "prores_proxy_half", "destination": destination.to_string_lossy()}));
        cleanup.jobs = s.jobs.clone();
        let second = second.unwrap();
        let next = second["outputs"][0]["path"].as_str().unwrap().to_owned();
        assert_eq!(second["outputs"][0]["item"], json!(items[2].0));
        assert_eq!(cleanup.jobs.len(), 2);
        assert_ne!(first["job"], second["job"], "both commands start real independent jobs");
        // The command returns the selected destination before its background job completes.
        // Check allocation while the first decoder is gated, then release it before draining:
        // waiting for another render here could block a single-worker Rayon pool.
        let first_unresolved = cleanup.jobs[0].result.lock().unwrap().is_none();
        eprintln!(
            "proxy allocation: first_job={} first_unresolved={first_unresolved} pending={pending:?} second_job={} next={next:?}",
            first["job"], second["job"]
        );
        assert!(first_unresolved, "first job remains unresolved after the second command selects its destination");
        assert_ne!(pending, next, "a running job must reserve its unwritten proxy output for its own take");
        cleanup.release();
        assert!(drain(&cleanup.jobs), "both real jobs must publish terminal results before final output assertions or cleanup");
        for job in &cleanup.jobs {
            let succeeded = job.result.lock().unwrap().as_ref().unwrap().is_ok();
            assert!(succeeded, "{}", job.to_json());
        }
        s.poll_persistence();
        for (((item, path), original), output) in items[1..].iter().zip(&paths).zip(&originals).zip([&pending, &next]) {
            assert_eq!(&std::fs::read(path).unwrap(), original, "source card remains unchanged");
            assert_eq!(
                s.project.item(*item).unwrap().as_media().unwrap().proxy,
                Some(MediaRef::File { path: output.clone() }),
                "finished job attaches its own planned proxy"
            );
        }
        // Reopen from disk so cached sources cannot hide one take overwriting the other.
        for (path, size) in [(&pending, (96, 48)), (&next, (64, 48))] {
            let proxy = crate::MediaPool::default().open_file(path, &*s.services).unwrap();
            let frame = proxy.video_frame(FrameRequest::full(filmcraft_time::Tick::ZERO)).unwrap();
            assert_eq!((frame.width, frame.height), size, "each proxy retains its own source dimensions at the chosen half-size preset");
        }
    }

    #[test]
    fn canceled_job_releases_unwritten_proxy_path_before_poll() {
        let root = tmp_dir("proxy-cancel-release");
        let (release, released) = mpsc::channel();
        let mut cleanup = Cleanup { root: root.clone(), release: Some(release), jobs: Vec::new() };
        let blocker = root.join("blocker.mov");
        let take = root.join("A001.mov");
        make_movie(&blocker, DemoScene::Forest, 64, 64, 4);
        make_movie(&take, DemoScene::Aurora, 192, 96, 4);
        let (mut s, items, _) = session_with(&[&blocker, &take]);
        let expected_error = format!("{}: cancelled", s.project.item(items[0]).unwrap().name);
        let source = s.media.full_res_source(&s.project, items[0], &*s.services).unwrap();
        let (entered, reached) = mpsc::channel();
        s.media.insert_file(items[0], &blocker.to_string_lossy(), Arc::new(PausedVideo { inner: source, entered, release: Mutex::new(Some(released)) }));
        let destination = root.join("Proxies");
        let first = s.execute(
            "media.createProxies",
            json!({"items": [items[0].0, items[1].0], "preset": "prores_proxy_half", "destination": destination.to_string_lossy()}),
        );
        cleanup.jobs = s.jobs.clone();
        let first = first.unwrap();
        reached.recv_timeout(Duration::from_secs(15)).expect("first job reached its real video decoder");
        let pending = first["outputs"][1]["path"].as_str().unwrap().to_owned();
        assert!(!std::path::Path::new(&pending).exists(), "later take has not started");
        s.execute("jobs.cancel", json!({"job": first["job"]})).unwrap();
        assert!(cleanup.jobs[0].progress.cancel.load(Ordering::Relaxed), "real command requested cancellation");
        cleanup.release();
        assert!(drain(&cleanup.jobs), "canceled producer must terminate before retry");
        // The paused ProRes batch returns Progress; the next step checks cancellation before
        // finishing. start_job prefixes that exact ExportError::Cancelled with the blocker name.
        let error = cleanup.jobs[0].result.lock().unwrap().as_ref().unwrap().as_ref().err().cloned();
        assert_eq!(error, Some(expected_error));
        assert!(!std::path::Path::new(&pending).exists(), "canceled job never wrote its later take");
        assert!(s.media_jobs.iter().any(|job| job.job == first["job"].as_u64().unwrap()), "no persistence poll has consumed the terminal job");

        let retry =
            s.execute("media.createProxies", json!({"items": [items[1].0], "preset": "prores_proxy_half", "destination": destination.to_string_lossy()}));
        cleanup.jobs = s.jobs.clone();
        let retry = retry.unwrap();
        let output = retry["outputs"][0]["path"].as_str().unwrap().to_owned();
        assert!(drain(&cleanup.jobs), "retry producer must finish before checking its output");
        assert_eq!(output, pending, "terminal cancellation releases the unused name without a UI poll");
        assert_ne!(retry["job"], first["job"], "retry is a new real job");
        let succeeded = cleanup.jobs.last().unwrap().result.lock().unwrap().as_ref().unwrap().is_ok();
        assert!(succeeded, "{}", cleanup.jobs.last().unwrap().to_json());
        let proxy = crate::MediaPool::default().open_file(&output, &*s.services).unwrap();
        let frame = proxy.video_frame(FrameRequest::full(filmcraft_time::Tick::ZERO)).unwrap();
        assert_eq!((frame.width, frame.height), (96, 48));
        s.poll_persistence();
        assert_eq!(s.project.item(items[1]).unwrap().as_media().unwrap().proxy, Some(MediaRef::File { path: output }));
    }

    #[test]
    fn cancellation_request_keeps_unwritten_proxy_path_reserved() {
        let root = tmp_dir("proxy-cancel-request-reservation");
        let (release, released) = mpsc::channel();
        let mut cleanup = Cleanup { root: root.clone(), release: Some(release), jobs: Vec::new() };
        let blocker = root.join("blocker.mov");
        let paths = [root.join("Card A/A001.mov"), root.join("Card B/A001.mov")];
        make_movie(&blocker, DemoScene::Forest, 64, 64, 4);
        for (path, scene, width) in [(&paths[0], DemoScene::Aurora, 192), (&paths[1], DemoScene::Plasma, 128)] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            make_movie(path, scene, width, 96, 4);
        }
        let originals: Vec<_> = paths.iter().map(|path| std::fs::read(path).unwrap()).collect();
        let (mut s, items, _) = session_with(&[&blocker, &paths[0], &paths[1]]);
        let expected_error = format!("{}: cancelled", s.project.item(items[0]).unwrap().name);
        let source = s.media.full_res_source(&s.project, items[0], &*s.services).unwrap();
        let (entered, reached) = mpsc::channel();
        s.media.insert_file(items[0], &blocker.to_string_lossy(), Arc::new(PausedVideo { inner: source, entered, release: Mutex::new(Some(released)) }));
        let destination = root.join("Proxies");
        let first = s.execute(
            "media.createProxies",
            json!({"items": [items[0].0, items[1].0], "preset": "prores_proxy_half", "destination": destination.to_string_lossy()}),
        );
        cleanup.jobs = s.jobs.clone();
        let first = first.unwrap();
        reached.recv_timeout(Duration::from_secs(15)).expect("first job reached its real video decoder");
        let pending = first["outputs"][1]["path"].as_str().unwrap().to_owned();
        assert_eq!(first["outputs"][1]["item"], json!(items[1].0));
        assert!(!std::path::Path::new(&pending).exists(), "later take has no output file yet");
        s.execute("jobs.cancel", json!({"job": first["job"]})).unwrap();
        assert!(cleanup.jobs[0].progress.cancel.load(Ordering::Relaxed), "the real command requested cancellation");
        assert!(cleanup.jobs[0].result.lock().unwrap().is_none(), "a request is not a terminal result");

        let second =
            s.execute("media.createProxies", json!({"items": [items[2].0], "preset": "prores_proxy_half", "destination": destination.to_string_lossy()}));
        cleanup.jobs = s.jobs.clone();
        let second = second.unwrap();
        let next = second["outputs"][0]["path"].as_str().unwrap().to_owned();
        assert_eq!(second["outputs"][0]["item"], json!(items[2].0));
        assert_ne!(first["job"], second["job"], "both commands start real independent jobs");
        assert_eq!(cleanup.jobs.len(), 2);
        // Destination selection is synchronous; a cancellation request must not free the
        // first job's name while its decoder is still gated. Release before awaiting renders.
        let first_unresolved = cleanup.jobs[0].result.lock().unwrap().is_none();
        eprintln!(
            "proxy allocation: first_job={} first_unresolved={first_unresolved} pending={pending:?} second_job={} next={next:?}",
            first["job"], second["job"]
        );
        assert!(first_unresolved, "the first producer remains unresolved when the second command selects its destination");
        assert_ne!(pending, next, "a cancellation request must retain an unresolved job's planned output");
        cleanup.release();
        assert!(drain(&cleanup.jobs), "both producers must terminate before final output assertions or cleanup");
        let error = cleanup.jobs[0].result.lock().unwrap().as_ref().unwrap().as_ref().err().cloned();
        assert_eq!(error, Some(expected_error));
        let succeeded = cleanup.jobs[1].result.lock().unwrap().as_ref().unwrap().is_ok();
        assert!(succeeded, "{}", cleanup.jobs[1].to_json());
        assert!(!std::path::Path::new(&pending).exists(), "the cancelled producer never wrote its later take");
        s.poll_persistence();
        assert_eq!(s.project.item(items[1]).unwrap().as_media().unwrap().proxy, None, "the cancelled take remains unattached");
        assert_eq!(
            s.project.item(items[2]).unwrap().as_media().unwrap().proxy,
            Some(MediaRef::File { path: next.clone() }),
            "the successful producer attaches its own proxy"
        );
        for (path, original) in paths.iter().zip(&originals) {
            assert_eq!(&std::fs::read(path).unwrap(), original, "both original takes remain unchanged");
        }
        let proxy = crate::MediaPool::default().open_file(&next, &*s.services).unwrap();
        let frame = proxy.video_frame(FrameRequest::full(filmcraft_time::Tick::ZERO)).unwrap();
        assert_eq!((frame.width, frame.height), (64, 48), "the fresh proxy decode belongs to the second take");
    }
}

#[test]
fn create_attach_toggle_renders_the_same_picture() {
    let root = tmp_dir("proxy-create");
    let a = root.join("a.mov");
    make_movie(&a, DemoScene::OceanSunset, W, H, 12);
    let (mut s, items, _) = session_with(&[&a]);
    add_effects(&mut s);
    let full = frame_rgba(&mut s, 5, 1.0);
    let full_half = frame_rgba(&mut s, 5, 0.5);
    let r = s.execute("media.createProxies", json!({"items": [items[0].0], "preset": "prores_proxy_half", "wait": true})).unwrap();
    let out = r["outputs"][0]["path"].as_str().unwrap().to_string();
    assert!(std::path::Path::new(&out).ends_with(std::path::Path::new("Proxies").join("a_Proxy.mov")), "{out}");
    let job = s.execute("jobs.list", json!({})).unwrap();
    assert_eq!(job[0]["finished"], json!(true), "{job}");
    let m = s.project.item(items[0]).unwrap().as_media().unwrap().clone();
    assert_eq!(m.proxy, Some(filmcraft_project::MediaRef::File { path: out.clone() }), "attached when the job finished");
    // proxies off: unchanged
    assert_eq!(frame_rgba(&mut s, 5, 1.0), full);
    let rev = s.revision;
    let dirty = s.is_dirty();
    s.execute("media.toggleProxies", json!({"enabled": true})).unwrap();
    assert!(s.revision > rev, "frame caches refresh");
    assert_eq!(s.is_dirty(), dirty, "toggling proxies is not an edit");
    let px = frame_rgba(&mut s, 5, 1.0);
    assert_eq!((px.0, px.1), (full.0, full.1));
    assert_ne!(px.2, full.2, "the proxy is really used");
    let q = psnr(&px.2, &full.2);
    let q_half = psnr(&frame_rgba(&mut s, 5, 0.5).2, &full_half.2);
    eprintln!("proxy vs full: {q:.1} dB at full scale, {q_half:.1} dB at ½");
    assert!(q > 30.0, "proxy composite matches full res modulo resolution: {q:.1} dB");
    assert!(q_half > 33.0, "at ½ playback resolution the proxy is near-identical: {q_half:.1} dB");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn attach_checks_duration_and_export_ignores_proxies() {
    let root = tmp_dir("proxy-attach");
    let a = root.join("a.mov");
    make_movie(&a, DemoScene::Aurora, W, H, 12);
    let short = root.join("short.mov");
    make_movie(&short, DemoScene::Aurora, W / 2, H / 2, 6);
    // a "proxy" with other pictures but the right duration and rate
    let other = root.join("other.mp4");
    make_movie(&other, DemoScene::Plasma, W / 4, H / 4, 12);
    let (mut s, items, _) = session_with(&[&a]);
    let e = s.execute("media.attachProxies", json!({"item": items[0].0, "path": short.to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("long"), "{e}");
    s.execute("media.attachProxies", json!({"item": items[0].0, "path": other.to_string_lossy()})).unwrap();
    let full = frame_rgba(&mut s, 4, 1.0);
    s.execute("media.toggleProxies", json!({"enabled": true})).unwrap();
    assert!(psnr(&frame_rgba(&mut s, 4, 1.0).2, &full.2) < 20.0, "monitors show the (different) proxy");
    let png = root.join("out.png").to_string_lossy().into_owned();
    s.execute("file.exportMedia", json!({"path": png, "format": "png", "wait": true})).unwrap();
    let jobs = s.execute("jobs.list", json!({})).unwrap();
    assert!(jobs.as_array().unwrap().iter().all(|j| j["result"].get("error").is_none()), "{jobs}");
    let f = image::open(root.join("out004.png")).unwrap().to_rgba8();
    assert_eq!(psnr(f.as_raw(), &full.2), f64::INFINITY, "export renders full-resolution media");
    s.execute("media.detachProxies", json!({"items": [items[0].0]})).unwrap();
    assert_eq!(frame_rgba(&mut s, 4, 1.0), full);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn ingest_copies_and_creates_proxies_on_import() {
    let root = tmp_dir("proxy-ingest");
    let cards = root.join("Card");
    std::fs::create_dir_all(&cards).unwrap();
    let a = cards.join("A001.mov");
    make_movie(&a, DemoScene::Forest, W, H, 8);
    let dest = root.join("Ingest");
    let mut s = Session::default();
    s.execute(
        "project.ingestSettings",
        json!({"enabled": true, "action": "copyAndCreateProxies", "destination": dest.to_string_lossy(), "preset": "h264_quarter"}),
    )
    .unwrap();
    let r = s.execute("file.import", json!({"paths": [a.to_string_lossy()]})).unwrap();
    let item = filmcraft_project::ItemId(r["items"][0].as_u64().unwrap());
    let job = r["ingest"]["job"]["job"].as_u64().unwrap_or_else(|| panic!("{r}"));
    // wait for the background proxy job, then let the session apply it
    for _ in 0..600 {
        if s.jobs.iter().any(|j| j.id == job && j.progress.finished.load(std::sync::atomic::Ordering::Relaxed)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    s.poll_persistence();
    let m = s.project.item(item).unwrap().as_media().unwrap();
    let copy = dest.join("A001.mov").to_string_lossy().into_owned();
    assert_eq!(m.media, filmcraft_project::MediaRef::File { path: copy.clone() }, "the clip uses the verified copy");
    assert_eq!(m.identity, Some(crate::relink::identity_of(&crate::FsServices, &a.to_string_lossy()).unwrap()));
    assert_eq!(m.proxy, Some(filmcraft_project::MediaRef::File { path: dest.join("A001_Proxy.mp4").to_string_lossy().into_owned() }));
    let _ = std::fs::remove_dir_all(&root);
}

/// Import two different camera takes with the same basename into a folder that already has
/// that name. Check the saved project with a new media pool so cached decoders cannot hide a
/// replaced file.
fn check_ingest_collision(action: &str) {
    use filmcraft_media::FrameRequest;
    use filmcraft_project::{ItemId, MediaRef};
    use filmcraft_time::FrameRate;
    use std::collections::BTreeSet;
    use std::sync::atomic::Ordering;

    let root = tmp_dir(&format!("ingest-collision-{action}"));
    let paths = [root.join("Card A/A001.mov"), root.join("Card B/A001.mov")];
    for (path, scene) in paths.iter().zip([DemoScene::Aurora, DemoScene::Plasma]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        make_movie(path, scene, 160, 96, 4);
    }
    let original: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
    assert_ne!(original[0], original[1], "different source takes");
    let dest = root.join("Ingest");
    std::fs::create_dir_all(&dest).unwrap();
    let existing = dest.join("A001.mov");
    let existing_bytes = b"existing destination must survive ingest";
    std::fs::write(&existing, existing_bytes).unwrap();
    let mut s = Session::default();
    s.execute("project.ingestSettings", json!({"enabled": true, "action": action, "destination": dest.to_string_lossy(), "preset": "prores_proxy_half"}))
        .unwrap();
    let result = s.execute("file.import", json!({"paths": paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>()})).unwrap();
    assert_eq!(result["errors"], json!([]), "{result}");
    let items: Vec<ItemId> = result["items"].as_array().unwrap().iter().map(|i| ItemId(i.as_u64().unwrap())).collect();
    assert_eq!(items.len(), 2, "both takes imported");
    if let Some(id) = result["ingest"]["job"]["job"].as_u64() {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let job = s.jobs.iter().find(|j| j.id == id).unwrap();
            if job.progress.finished.load(Ordering::Relaxed) && job.result.lock().unwrap().is_some() {
                assert!(job.result.lock().unwrap().as_ref().unwrap().is_ok(), "ingest job failed");
                break;
            }
            assert!(std::time::Instant::now() < deadline, "ingest job did not finish");
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        s.poll_persistence();
    }
    assert_eq!(std::fs::read(&existing).unwrap(), existing_bytes, "ingest must preserve existing destination files");
    let project_path = root.join("ingested.fcproj");
    s.execute("file.save", json!({"path": project_path.to_string_lossy()})).unwrap();
    let mut reopened = Session::default();
    reopened.execute("file.open", json!({"path": project_path.to_string_lossy()})).unwrap();
    let request = FrameRequest { time: FrameRate::FPS_24.tick_of(2), scale: 1.0 };
    let mut linked = BTreeSet::new();
    let mut proxies = BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
        let media = reopened.project.item(*item).unwrap().as_media().unwrap();
        let MediaRef::File { path } = &media.media else { panic!("ingest must retain file media") };
        assert!(linked.insert(path.clone()), "different takes must have different ingest paths");
        assert_eq!(std::path::Path::new(path).parent(), Some(dest.as_path()));
        assert_eq!(std::fs::read(&paths[index]).unwrap(), original[index], "source card is unchanged");
        if action != "transcode" {
            assert_eq!(std::fs::read(path).unwrap(), original[index], "each copy retains its own take");
            assert_eq!(media.identity, Some(crate::relink::identity_of_bytes(&original[index])));
        }
        let source = reopened.media.open_file(&paths[index].to_string_lossy(), &*reopened.services).unwrap();
        let expected = source.video_frame(request).unwrap();
        let ingested = reopened.source(*item).unwrap().video_frame(request).unwrap();
        let quality = |frame: &filmcraft_frame::VideoFrame| {
            let rgba = image::RgbaImage::from_raw(frame.width, frame.height, frame.to_rgba8()).unwrap();
            let expected = image::RgbaImage::from_raw(expected.width, expected.height, expected.to_rgba8()).unwrap();
            let expected = image::imageops::resize(&expected, frame.width, frame.height, image::imageops::FilterType::Triangle);
            psnr(rgba.as_raw(), expected.as_raw())
        };
        assert!(quality(&ingested) > 30.0, "saved project retains the correct picture for take {index}");
        if action == "copyAndCreateProxies" {
            let Some(MediaRef::File { path }) = &media.proxy else { panic!("copy should have its proxy attached") };
            assert!(proxies.insert(path.clone()), "different takes must have different proxy paths");
            let proxy = reopened.media.open_file(path, &*reopened.services).unwrap().video_frame(request).unwrap();
            assert!(quality(&proxy) > 30.0, "proxy retains the correct picture for take {index}");
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn ingest_copy_preserves_same_named_takes_and_existing_files() {
    check_ingest_collision("copy");
}

#[test]
fn ingest_copy_and_proxy_preserves_same_named_takes_and_existing_files() {
    check_ingest_collision("copyAndCreateProxies");
}

#[test]
fn ingest_transcode_preserves_same_named_takes_and_existing_files() {
    check_ingest_collision("transcode");
}

#[test]
fn ingest_copy_keeps_sources_already_at_destination_and_extensionless_names() {
    use filmcraft_project::{ItemId, MediaRef};

    for (name, numbered) in [("Take.wav", "Take 2.wav"), ("Take", "Take 2")] {
        let root = tmp_dir("ingest-copy-existing-source");
        let dest = root.join("Ingest");
        let card = root.join("Card");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::create_dir_all(&card).unwrap();
        let paths = [dest.join(name), card.join(name)];
        let takes = [filmcraft_media::wav::write_wav16(&[0.25; 960], 1, 48_000), filmcraft_media::wav::write_wav16(&[-0.25; 960], 1, 48_000)];
        for (path, bytes) in paths.iter().zip(&takes) {
            std::fs::write(path, bytes).unwrap();
        }
        let mut s = Session::default();
        s.execute("project.ingestSettings", json!({"enabled": true, "action": "copy", "destination": dest.to_string_lossy()})).unwrap();
        let result = s.execute("file.import", json!({"paths": paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>()})).unwrap();
        assert_eq!(result["errors"], json!([]), "{result}");
        assert_eq!(result["items"].as_array().unwrap().len(), 2);
        assert_eq!(result["ingest"]["copied"].as_array().unwrap().len(), 1, "source already at destination is kept");
        for (index, path) in [dest.join(name), dest.join(numbered)].iter().enumerate() {
            let item = ItemId(result["items"][index].as_u64().unwrap());
            let media = s.project.item(item).unwrap().as_media().unwrap();
            assert_eq!(media.media, MediaRef::File { path: path.to_string_lossy().into_owned() });
            assert_eq!(std::fs::read(path).unwrap(), takes[index]);
            assert_eq!(std::fs::read(&paths[index]).unwrap(), takes[index]);
        }
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 2, "no unnecessary copies or trailing-dot filenames");
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn ingest_copy_does_not_create_a_destination_when_the_source_is_missing() {
    use filmcraft_project::ItemId;

    let root = tmp_dir("ingest-copy-missing-source");
    let source = root.join("Take.wav");
    std::fs::write(&source, filmcraft_media::wav::write_wav16(&[0.25; 960], 1, 48_000)).unwrap();
    let mut s = Session::default();
    let imported = s.execute("file.import", json!({"paths": [source.to_string_lossy()]})).unwrap();
    let item = ItemId(imported["items"][0].as_u64().unwrap());
    let dest = root.join("Ingest");
    s.execute("project.ingestSettings", json!({"enabled": true, "action": "copy", "destination": dest.to_string_lossy()})).unwrap();
    let before = s.project.clone();
    std::fs::rename(&source, root.join("Moved.wav")).unwrap();
    assert!(crate::proxies::ingest(&mut s, &[item]).is_err());
    assert_eq!(s.project, before, "failed copy does not relink the item");
    assert!(!dest.exists(), "no destination is created before the source can be opened");
    let _ = std::fs::remove_dir_all(root);
}

/// Measure: Program-monitor playback cost of 4K H.264 with and without proxies (sequential frames
/// at ½ resolution). Run with `RAYON_NUM_THREADS=1` so wall time ≈ CPU time:
/// `cargo test --release -p filmcraft-engine proxy_playback_perf_4k -- --ignored --nocapture`.
#[test]
#[ignore = "benchmark: 4K fixture via ffmpeg"]
fn proxy_playback_perf_4k() {
    let Some(ffmpeg) = filmcraft_testkit::oracle::ffmpeg_or_skip("proxy_playback_perf_4k") else { return };
    let out = filmcraft_testkit::fixtures_dir("proxies").join("uhd_h264.mp4");
    let src = filmcraft_testkit::fixtures::generate(&out, |tmp| {
        std::process::Command::new(&ffmpeg)
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=3840x2160:rate=24:duration=2",
                "-c:v",
                "libx264",
                "-preset",
                "fast",
                "-pix_fmt",
                "yuv420p",
                "-g",
                "48",
            ])
            .arg(tmp)
            .status()
            .is_ok_and(|s| s.success())
    })
    .expect("fixture");
    let dir = tmp_dir("proxy-perf");
    let media = dir.join("uhd.mp4");
    std::fs::copy(&src, &media).unwrap();
    let (mut s, items, _) = session_with(&[&media]);
    let n = 24;
    // (decode ms/frame as the GPU playback path pays it, CPU composite ms/frame at ½ and ¼)
    let measure = |s: &mut Session| {
        let rate = s.sequence_rate();
        let src = s.source(items[0]).unwrap();
        let _ = src.video_frame(filmcraft_media::FrameRequest { time: rate.tick_of(0), scale: 0.5 });
        let t0 = std::time::Instant::now();
        for f in 1..n {
            let _ = src.video_frame(filmcraft_media::FrameRequest { time: rate.tick_of(f), scale: 0.5 }).unwrap();
        }
        let decode = t0.elapsed().as_secs_f64() * 1000.0 / (n - 1) as f64;
        let mut comp = [0.0; 2];
        for (k, scale) in [0.5f32, 0.25].into_iter().enumerate() {
            s.media.clear();
            frame_rgba(s, 0, scale);
            let t0 = std::time::Instant::now();
            for f in 1..n {
                frame_rgba(s, f, scale);
            }
            comp[k] = t0.elapsed().as_secs_f64() * 1000.0 / (n - 1) as f64;
        }
        (decode, comp[0], comp[1])
    };
    let full = measure(&mut s);
    eprintln!("4K H.264, full res: decode {:.1} ms/frame; CPU composite {:.1} ms/frame at 1/2, {:.1} at 1/4", full.0, full.1, full.2);
    for preset in ["prores_proxy_quarter", "h264_quarter"] {
        let t0 = std::time::Instant::now();
        s.execute("media.createProxies", json!({"items": [items[0].0], "preset": preset, "wait": true})).unwrap();
        let make = t0.elapsed().as_secs_f64();
        s.execute("media.toggleProxies", json!({"enabled": true})).unwrap();
        let px = measure(&mut s);
        s.execute("media.toggleProxies", json!({"enabled": false})).unwrap();
        eprintln!(
            "{preset} proxy (made in {make:.1}s): decode {:.1} ms/frame ({:.1}x less); CPU composite {:.1} ms/frame at 1/2 ({:.1}x less), {:.1} at 1/4 ({:.1}x less)",
            px.0,
            full.0 / px.0,
            px.1,
            full.1 / px.1,
            px.2,
            full.2 / px.2
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Render previews are cached by content and reused whether proxies are on or off, so they are
/// always rendered from full-resolution media (like export).
#[test]
fn render_previews_ignore_proxies() {
    let root = tmp_dir("proxy-previews");
    let a = root.join("a.mov");
    make_movie(&a, DemoScene::Aurora, W, H, 12);
    let other = root.join("other.mp4");
    make_movie(&other, DemoScene::Plasma, W / 4, H / 4, 12);
    let (mut s, items, _) = session_with(&[&a]);
    add_effects(&mut s);
    let full = frame_rgba(&mut s, 4, 1.0);
    s.execute("media.attachProxies", json!({"item": items[0].0, "path": other.to_string_lossy()})).unwrap();
    s.execute("media.toggleProxies", json!({"enabled": true})).unwrap();
    s.execute("sequence.renderInToOut", json!({"wait": true})).unwrap();
    let seq = s.state.active_sequence.unwrap();
    let f = s.previews.frame(&s.media, &s.project, seq, 4, 1.0).expect("frame 4 has a preview");
    let q = psnr(&f.to_rgba8(), &full.2);
    assert!(q > 30.0, "preview rendered from full-resolution media, not the (different) proxy: {q:.1} dB");
    let _ = std::fs::remove_dir_all(&root);
}
