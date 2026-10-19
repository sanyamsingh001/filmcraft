//! Frame caches obey one Memory budget, and abandoned frame servers release their workers.

use super::{FrameCache, FrameKey, FrameServer, FrameView, GpuPlan, Job, JobDemand, Rgba, Target, ViewId, WorkingFrame};
use filmcraft_project::ItemId;
use std::sync::Arc;

fn key(frame: i64, plan: bool) -> FrameKey {
    FrameKey { target: if plan { Target::SequencePlan(ItemId(1)) } else { Target::Sequence(ItemId(1)) }, frame, size: 1000, revision: 0, draft: false }
}

fn image(bytes: usize) -> Arc<Rgba> {
    Arc::new(Rgba { w: bytes / 4, h: 1, px: vec![0; bytes] })
}

fn plan(samples: usize) -> GpuPlan {
    let plan = filmcraft_render::plan::FramePlan::Image(filmcraft_render::Image { w: samples / 4, h: 1, px: vec![0.0; samples] });
    let prepared = filmcraft_gpu::prepare(&plan);
    GpuPlan { plan, prepared }
}

#[test]
fn images_and_gpu_plans_share_a_budget_and_use_order() {
    let mut cache = FrameCache::new(240);
    let (old_image, gpu_plan, new_image) = (key(0, false), key(1, true), key(2, false));
    cache.insert(old_image, image(80));
    cache.insert_plan(gpu_plan, plan(16)); // 64 bytes of floats + 32 prepared bytes
    assert_eq!(cache.bytes(), 176);
    assert!(cache.get(&old_image).is_some());
    cache.insert(new_image, image(80));
    assert!(cache.get_plan(&gpu_plan).is_none(), "the image touched last is newer than the plan");
    assert!(cache.get(&old_image).is_some() && cache.get(&new_image).is_some());
    assert_eq!(cache.bytes(), 160);

    let newest_plan = key(3, true);
    cache.insert_plan(newest_plan, plan(16));
    assert!(cache.get(&old_image).is_none(), "LRU eviction crosses cache kinds in both directions");
    assert!(cache.get(&new_image).is_some() && cache.get_plan(&newest_plan).is_some());
    assert_eq!(cache.bytes(), 176);
}

#[test]
fn eviction_headroom_keeps_a_frame_that_fits_the_budget() {
    let mut cache = FrameCache::new(100);
    cache.insert(key(0, false), image(40));
    cache.insert(key(1, false), image(90));
    assert!(cache.get(&key(0, false)).is_none());
    assert!(cache.get(&key(1, false)).is_some());
    assert_eq!(cache.bytes(), 90);
}

fn job(key: FrameKey, prefetch: bool, prio: u32) -> Job {
    Job {
        key,
        queued: web_time::Instant::now(),
        cancel: Default::default(),
        demand: JobDemand { prefetch, prio, views: vec![ViewId::Anonymous(key.target, key.size, if prefetch { 0 } else { prio })] },
        prefetch_epoch: 0,
        catch_up: None,
        time: filmcraft_time::Tick::ZERO,
        scale: 1.0,
        project: Arc::new(filmcraft_project::Project::new("working frame")),
    }
}

fn publish_plan(cache: &mut FrameCache, job: &Job, plan: GpuPlan) {
    cache.publish_plan(job.key, &job.demand, plan);
}

fn view_job(key: FrameKey, view: FrameView, prefetch: bool, prio: u32) -> Job {
    let mut job = job(key, prefetch, prio);
    job.demand.views = vec![ViewId::Named(view)];
    job
}

#[test]
fn oversized_handoff_is_bounded_and_prefetch_preserves_demanded_frames() {
    let mut cache = FrameCache::new(16);
    let current = key(0, true);
    let reference = key(1, true);
    publish_plan(&mut cache, &job(current, false, 0), plan(16));
    publish_plan(&mut cache, &job(reference, false, 1), plan(16));
    for frame in 2..20 {
        publish_plan(&mut cache, &job(key(frame, true), true, 1), plan(16));
    }
    assert!(cache.get_plan(&current).is_some() && cache.get_plan(&reference).is_some());
    assert!(cache.get_plan(&key(19, true)).is_some());
    assert_eq!(cache.working.len(), 3, "only the latest prefetch result is held");
    assert_eq!(cache.bytes(), 0, "working frames are not retained cache entries");
    for target in 2..20 {
        let k = FrameKey { target: Target::SequencePlan(ItemId(target)), ..key(0, true) };
        publish_plan(&mut cache, &job(k, true, 1), plan(16));
        assert!(cache.working.len() <= FrameCache::MAX_BACKGROUND);
    }
    assert!(cache.get_plan(&current).is_some() && cache.get_plan(&reference).is_some());
    cache.clear();
    assert!(cache.working.is_empty());
}

#[test]
fn prefetched_oversized_frame_becomes_the_demanded_result_and_releases_when_replaced() {
    let mut cache = FrameCache::new(32);
    let prefetched = key(1, true);
    publish_plan(&mut cache, &job(prefetched, true, 1), plan(16));
    cache.demand_ready(prefetched, ViewId::Anonymous(prefetched.target, prefetched.size, 0));
    publish_plan(&mut cache, &job(key(2, true), true, 1), plan(16));
    assert!(cache.get_plan(&prefetched).is_some(), "next prefetch cannot overwrite the newly demanded result");
    publish_plan(&mut cache, &job(key(3, true), false, 0), plan(4));
    assert!(cache.get_plan(&prefetched).is_none(), "a smaller replacement releases the oversized working frame");
    assert!(cache.get_plan(&key(3, true)).is_some());
    assert_eq!(cache.bytes(), 24);
    assert_eq!(cache.working.len(), 1, "only the independent prefetch remains");
}

#[test]
fn secondary_views_and_trim_sides_keep_independent_oversized_results() {
    let mut cache = FrameCache::new(16);
    let views = [FrameView::Comparison, FrameView::Reference, FrameView::ProgramScopes, FrameView::ReferenceScopes];
    for (i, view) in views.into_iter().enumerate() {
        let k = FrameKey { size: if i < 2 { 1000 } else { 250 }, ..key(i as i64, false) };
        let job = view_job(k, view, false, if i == 0 { 1 } else { 3 });
        cache.publish_image(k, &job.demand, image(32));
    }
    for frame in 0..4 {
        let k = FrameKey { size: if frame < 2 { 1000 } else { 250 }, ..key(frame, false) };
        assert!(cache.get(&k).is_some(), "another secondary view cannot remove this picture");
    }
    assert_eq!(cache.working.len(), views.len());

    cache.clear();
    let source_key = FrameKey { target: Target::Item(ItemId(2)), ..key(0, false) };
    for (i, view) in [FrameView::Source, FrameView::TrimLeft, FrameView::TrimRight].into_iter().enumerate() {
        let k = FrameKey { frame: i as i64, ..source_key };
        cache.publish_image(k, &view_job(k, view, false, 0).demand, image(32));
    }
    for frame in 0..3 {
        assert!(cache.get(&FrameKey { frame, ..source_key }).is_some(), "equal target, size and priority still belong to different consumers");
    }
}

#[test]
fn five_displayed_consumers_keep_results_with_separately_bounded_prefetch_and_visible_metrics() {
    let app = crate::FilmcraftApp::new(filmcraft_engine::Session::default());
    let server = app.frames.clone();
    server.shared.cache.lock().unwrap().budget = 16;
    let views = [FrameView::Source, FrameView::Program, FrameView::Comparison, FrameView::Reference, FrameView::ProgramScopes];
    let visible_pass = server.begin_view_pass();
    let mut weak = Vec::new();
    for (i, view) in views.into_iter().enumerate() {
        let frame_key = key(i as i64, false);
        let frame = image(32);
        weak.push(Arc::downgrade(&frame));
        server.shared.cache.lock().unwrap().publish_image(frame_key, &view_job(frame_key, view, false, 0).demand, frame);
        server.request_for(view, frame_key, filmcraft_time::Tick::ZERO, 1.0, &app.session.project, 0);
    }
    for target in 2..30 {
        let frame_key = FrameKey { target: Target::Item(ItemId(target)), ..key(10, false) };
        let prefetch = job(frame_key, true, 1);
        server.shared.cache.lock().unwrap().publish_image(frame_key, &prefetch.demand, image(32));
    }
    for (i, frame) in weak.iter().enumerate() {
        assert!(frame.upgrade().is_some() && server.is_ready(&key(i as i64, false)), "prefetch cannot evict any displayed consumer");
    }
    let count = views.len() + FrameCache::MAX_BACKGROUND;
    assert_eq!(server.working_usage(), (count, count * 32));
    assert_eq!(server.cache_usage(), (0, 16));
    let stats = crate::perf::stats(&app);
    assert_eq!(stats["frames"]["working"]["count"], serde_json::json!(count));
    assert_eq!(stats["frames"]["working"]["MB"], serde_json::json!((count * 32) as f64 / 1e6));
    drop(visible_pass);
    assert_eq!(server.working_usage().0, count, "visible consumers survive UI pass retirement");

    let hidden_pass = server.begin_view_pass();
    server.request_for(FrameView::Source, key(0, false), filmcraft_time::Tick::ZERO, 1.0, &app.session.project, 0);
    drop(hidden_pass);
    assert!(weak[0].upgrade().is_some());
    assert!(weak[1..].iter().all(|frame| frame.upgrade().is_none()), "hiding the other consumers releases their memory");
    server.stop_prefetch();
    assert_eq!(server.working_usage(), (1, 32));
    drop(server.begin_view_pass()); // No consumers visible, including on an early return.
    assert!(weak[0].upgrade().is_none());
    assert_eq!(server.working_usage(), (0, 0));
}

#[test]
fn a_view_replacement_releases_previous_target_and_resolution_on_retained_cache_hit() {
    let server = server();
    let old = key(0, true);
    let small = FrameKey { target: Target::SequencePlan(ItemId(2)), size: 250, ..key(1, true) };
    let old_plan;
    {
        let mut cache = server.shared.cache.lock().unwrap();
        cache.budget = 32;
        publish_plan(&mut cache, &view_job(old, FrameView::Reference, false, 3), plan(16));
        old_plan = Arc::downgrade(&cache.get_plan(&old).unwrap());
        publish_plan(&mut cache, &view_job(small, FrameView::Reference, true, 4), plan(4));
    }
    assert!(old_plan.upgrade().is_some());
    server.request_for(FrameView::Reference, small, filmcraft_time::Tick::ZERO, 0.25, &Arc::new(filmcraft_project::Project::new("cached replacement")), 3);
    assert!(old_plan.upgrade().is_none(), "a ready small frame must immediately release this view's old oversized frame");
    assert!(server.is_ready(&small));
    assert_eq!(server.working_usage(), (0, 0));
    assert_eq!(server.stats().request_misses, 0);
}

#[test]
fn consumers_of_the_same_working_frame_can_replace_their_views_independently() {
    let mut cache = FrameCache::new(16);
    let same = key(0, false);
    let old = image(32);
    let weak = Arc::downgrade(&old);
    cache.publish_image(same, &view_job(same, FrameView::TrimLeft, false, 0).demand, old);
    cache.demand_ready(same, ViewId::Named(FrameView::TrimRight));
    assert_eq!(cache.working.len(), 2, "two consumers hold the shared picture");
    assert_eq!(cache.working_usage(), (1, 32), "one shared allocation is reported once");
    let left_next = FrameKey { frame: 1, size: 250, ..same };
    cache.publish_image(left_next, &view_job(left_next, FrameView::TrimLeft, false, 0).demand, image(32));
    assert!(weak.upgrade().is_some(), "the right side still owns the shared old picture");
    assert!(cache.get(&same).is_some() && cache.get(&left_next).is_some());
    let right_next = FrameKey { target: Target::Sequence(ItemId(2)), frame: 2, ..same };
    cache.publish_image(right_next, &view_job(right_next, FrameView::TrimRight, false, 0).demand, image(32));
    assert!(weak.upgrade().is_none(), "both consumers replaced the shared picture");
    assert_eq!(cache.working.len(), 2);
}

#[test]
fn replacement_updates_both_cache_byte_counts() {
    let mut cache = FrameCache::new(1024);
    cache.insert(key(0, false), image(80));
    cache.insert(key(0, false), image(40));
    cache.insert_plan(key(1, true), plan(16));
    cache.insert_plan(key(1, true), plan(8));
    assert_eq!((cache.image_bytes, cache.plan_bytes, cache.bytes()), (40, 48, 88));
    cache.clear();
    assert_eq!(cache.bytes(), 0);
}

#[test]
fn gpu_plan_count_remains_bounded_without_evicting_images() {
    let mut cache = FrameCache::new(1 << 20);
    let image_key = key(0, false);
    cache.insert(image_key, image(16));
    for frame in 0..=FrameCache::MAX_PLANS as i64 {
        cache.insert_plan(key(frame, true), plan(4));
    }
    assert_eq!(cache.plans.len(), FrameCache::MAX_PLANS);
    assert!(cache.get_plan(&key(0, true)).is_none());
    assert!(cache.get(&image_key).is_some());
    assert_eq!(cache.bytes(), 16 + FrameCache::MAX_PLANS * 24);
}

#[test]
fn distant_prefetch_publication_preserves_the_oversized_nearest_future() {
    for gpu in [false, true] {
        let mut cache = FrameCache::new(16);
        let current = view_job(key(0, gpu), FrameView::Program, false, 0);
        let nearest = view_job(key(1, gpu), FrameView::Program, true, 1);
        let distant = view_job(key(2, gpu), FrameView::Program, true, 2);
        if gpu {
            cache.publish_plan(current.key, &current.demand, plan(16));
            cache.publish_plan(nearest.key, &nearest.demand, plan(16));
            cache.publish_plan(distant.key, &distant.demand, plan(16));
        } else {
            cache.publish_image(current.key, &current.demand, image(32));
            cache.publish_image(nearest.key, &nearest.demand, image(32));
            cache.publish_image(distant.key, &distant.demand, image(32));
        }
        assert!(cache.is_ready(&current.key));
        assert!(cache.is_ready(&nearest.key), "already-running distant jobs must not replace the affordable next picture");
        assert!(!cache.is_ready(&distant.key));
        assert_eq!(cache.working_usage().0, 2);
        // A smaller distant result fits the LRU but also must preserve the nearer handoff.
        if !gpu {
            cache.publish_image(distant.key, &distant.demand, image(8));
            assert!(cache.is_ready(&distant.key) && cache.is_ready(&nearest.key));
            assert_eq!(cache.bytes(), 8);
            assert_eq!(cache.working_usage(), (2, 64));
        }
    }
}

#[test]
fn surplus_prefetch_skip_never_discards_a_promoted_demand() {
    let server = server();
    let current = view_job(key(0, false), FrameView::Program, false, 0);
    server.shared.cache.lock().unwrap().budget = 16;
    server.shared.cache.lock().unwrap().publish_image(current.key, &current.demand, image(32));
    let distant = view_job(key(2, false), FrameView::Program, true, 2);
    let demand = super::register_job(&server.shared, &distant).unwrap();
    server.request_for(FrameView::Program, distant.key, distant.time, distant.scale, &distant.project, 0);
    assert!(!demand.lock().unwrap().prefetch, "the actual request path promotes the registered job");
    assert!(!super::skip_surplus_prefetch(&server.shared, &distant, &demand));
    assert!(!distant.cancel.load(std::sync::atomic::Ordering::Relaxed));
    assert_eq!(server.shared.in_flight.lock().unwrap().len(), 1);
}

#[test]
fn playback_future_handles_extreme_ordinals_and_selects_the_normal_reverse_future() {
    let server = server();
    let view = ViewId::Named(FrameView::Program);
    for frame in [i64::MIN + 1, i64::MAX] {
        let job = view_job(key(frame, false), FrameView::Program, true, 1);
        assert!(super::register_job(&server.shared, &job).is_some());
    }
    assert_eq!(server.playback_future(key(i64::MIN, false), view, 1), (Some(key(i64::MIN + 1, false)), false));
    assert_eq!(server.playback_future(key(i64::MAX, false), view, -1), (Some(key(i64::MIN + 1, false)), true));
    server.shared.in_flight.lock().unwrap().clear();

    for frame in [8, 12] {
        let job = view_job(key(frame, false), FrameView::Program, true, 1);
        assert!(super::register_job(&server.shared, &job).is_some());
    }
    assert_eq!(server.playback_future(key(10, false), view, -1), (Some(key(8, false)), false), "reverse playback keeps the nearest frame below the playhead");
}

fn server() -> FrameServer {
    FrameServer::new(
        Arc::new(filmcraft_engine::MediaPool::default()),
        Arc::new(filmcraft_engine::FsServices),
        Arc::new(filmcraft_engine::previews::PreviewStore::default()),
        2,
    )
}

fn held_working_plan(server: &FrameServer, key: FrameKey, prefetch: bool) -> std::sync::Weak<GpuPlan> {
    let frame = Arc::new(plan(16));
    let weak = Arc::downgrade(&frame);
    server.shared.cache.lock().unwrap().handoff(key, prefetch, ViewId::Anonymous(key.target, key.size, 0), WorkingFrame::Plan(frame));
    weak
}

fn held_cached_plan(server: &FrameServer, key: FrameKey) -> std::sync::Weak<GpuPlan> {
    let mut cache = server.shared.cache.lock().unwrap();
    cache.insert_plan(key, plan(16));
    Arc::downgrade(&cache.get_plan(&key).unwrap())
}

#[test]
fn stopping_prefetch_releases_oversized_results_but_keeps_the_demanded_picture() {
    let server = server();
    let demanded = held_working_plan(&server, key(0, true), false);
    let prefetched = held_working_plan(&server, key(1, true), true);
    server.stop_prefetch();
    assert!(prefetched.upgrade().is_none());
    assert!(demanded.upgrade().is_some());
    assert_eq!(server.working_usage(), (1, 96));
    drop(server);
    assert!(demanded.upgrade().is_none());
}

#[test]
fn lowering_memory_budget_evicts_cached_images_and_plans_immediately() {
    let server = server();
    server.set_cache_budget(32 << 20);
    {
        let mut cache = server.shared.cache.lock().unwrap();
        cache.insert(key(0, false), image(12 << 20));
        cache.insert_plan(key(1, true), plan(2 << 20)); // 12 MiB including preparation
    }
    assert_eq!(server.cache_usage(), (24 << 20, 32 << 20));
    server.set_cache_budget(16 << 20);
    assert!(server.get(&key(0, false)).is_none(), "older CPU frame evicted immediately");
    assert!(server.get_plan(&key(1, true)).is_some());
    assert_eq!(server.cache_usage(), (12 << 20, 16 << 20));
    let (images, image_bytes, plans, plan_bytes) = server.cache_entries();
    assert_eq!((images, image_bytes, plans, plan_bytes), (0, 0, 1, 12 << 20));

    {
        let mut cache = server.shared.cache.lock().unwrap();
        cache.insert(key(2, false), image(12 << 20));
    }
    assert!(server.get_plan(&key(1, true)).is_none(), "GPU plans share the reduced budget on future inserts too");
    assert_eq!(server.cache_usage(), (12 << 20, 16 << 20));
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use filmcraft_media::{FrameRequest, Generator, MediaInfo, MediaSource};
    use filmcraft_project::{ItemKind, Label, MediaClip, MediaRef, Project};
    use filmcraft_time::{FrameRate, Tick};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc::{Sender, channel};
    use std::time::{Duration, Instant};

    fn wait_until(mut ready: impl FnMut() -> bool) -> bool {
        let until = Instant::now() + Duration::from_secs(2);
        while !ready() {
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        true
    }

    #[test]
    fn dropping_idle_server_releases_workers_caches_and_host_references() {
        // Repeated project changes must not keep another set of workers or caches each time.
        for _ in 0..8 {
            let server = server();
            let (pool, previews, services, shared, repaint) = (
                Arc::downgrade(&server.pool),
                Arc::downgrade(&server.previews),
                Arc::downgrade(&server.services),
                Arc::downgrade(&server.shared),
                Arc::downgrade(&server.repaint),
            );
            let cached = image(32);
            let cached_weak = Arc::downgrade(&cached);
            server.shared.cache.lock().unwrap().insert(key(0, false), cached);
            let cached_plan = held_cached_plan(&server, key(1, true));
            let working = held_working_plan(&server, key(2, true), false);
            drop(server);
            assert!(wait_until(|| pool.upgrade().is_none()
                && previews.upgrade().is_none()
                && services.upgrade().is_none()
                && shared.upgrade().is_none()
                && repaint.upgrade().is_none()));
            assert!(cached_weak.upgrade().is_none());
            assert!(cached_plan.upgrade().is_none() && working.upgrade().is_none());
        }
    }

    #[test]
    fn low_memory_budget_still_delivers_1080p_fallback_and_4k_cpu_frames() {
        use filmcraft_project::{SequenceSettings, TrackKind};
        use filmcraft_time::TimeRange;

        for gpu in [true, false] {
            let (w, h) = if gpu { (1920, 1080) } else { (3840, 2160) };
            let generator = Generator::ColorMatte { color: [1.0, 0.0, 0.0, 1.0] };
            let source = filmcraft_media::generators::GeneratorSource::new(generator.clone(), w, h, FrameRate::FPS_24, Tick::from_seconds_f64(1.0));
            let mut project = Project::new("oversized working frame");
            let item = project.add_item(
                "red",
                Label::Iris,
                ItemKind::Media(MediaClip {
                    media: MediaRef::Generator(generator),
                    info: source.info().clone(),
                    interpret: Default::default(),
                    mark_in: None,
                    mark_out: None,
                    markers: vec![],
                    offline: false,
                    proxy: None,
                    identity: None,
                }),
                None,
            );
            let mut settings = SequenceSettings { width: w, height: h, frame_rate: FrameRate::FPS_24, ..Default::default() };
            if gpu {
                // A supported HDR sequence exercises the real whole-frame CPU fallback plan.
                settings.color.working = filmcraft_color::WorkingSpace::Rec2100Pq;
            }
            let seq = project.new_sequence("picture", settings, 1, 0, None);
            let mut clip = project
                .make_track_item(item, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, Tick::from_seconds_f64(1.0)), FrameRate::FPS_24)
                .unwrap();
            for effect in &mut clip.effects {
                filmcraft_project::resolve_auto_points(effect, (w, h), (w, h));
            }
            project.sequence_mut(seq).unwrap().video_tracks[0].items.push(clip);
            let project = Arc::new(project);
            let mut session = filmcraft_engine::Session::default();
            session.project = project.clone();
            session.state.active_sequence = Some(seq);
            let app = crate::FilmcraftApp::new(session);
            let server = app.frames.clone();
            server.set_cache_budget(16 << 20);
            let frame_key = FrameKey { target: if gpu { Target::SequencePlan(seq) } else { Target::Sequence(seq) }, ..key(0, gpu) };
            server.request(frame_key, Tick::ZERO, 1.0, &project, 0);
            let until = Instant::now() + Duration::from_secs(20);
            while !server.is_ready(&frame_key) && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(server.is_ready(&frame_key), "a normal oversized picture must reach the monitor");
            let bytes = if gpu {
                let plan = server.get_plan(&frame_key).expect("GPU monitor receives fallback plan");
                let filmcraft_render::plan::FramePlan::Image(image) = &plan.plan else { panic!("HDR sequence must use CPU fallback") };
                assert_eq!((image.w, image.h), (w as usize, h as usize));
                assert!(image.px[0] > 0.1 && image.px[0] > image.px[1] * 3.0 && image.px[0] > image.px[2] * 3.0, "red footage rendered");
                plan.bytes()
            } else {
                let image = server.get(&frame_key).expect("CPU monitor receives frame");
                assert_eq!((image.w, image.h), (w as usize, h as usize));
                assert_eq!(&image.px[..4], &[255, 0, 0, 255]);
                image.px.len()
            };
            assert!(bytes > 16 << 20);
            assert_eq!(server.cache_usage(), (0, 16 << 20));
            assert_eq!(server.working_usage(), (1, bytes));
            let stats = crate::perf::stats(&app);
            assert_eq!(stats["frames"]["working"]["count"], serde_json::json!(1));
            assert_eq!(stats["frames"]["working"]["MB"], serde_json::json!(bytes as f64 / 1e6));
            assert_eq!(stats["frames"]["cache"]["imageMB"], serde_json::json!(0.0));
            assert_eq!(stats["frames"]["cache"]["planMB"], serde_json::json!(0.0));
            let misses = server.stats().request_misses;
            server.request(frame_key, Tick::ZERO, 1.0, &project, 0);
            assert_eq!(server.stats().request_misses, misses, "paused monitors do not endlessly rerender an oversized result");
        }
    }

    struct AnalysisFrames {
        info: MediaInfo,
        reads: Arc<AtomicUsize>,
    }

    impl MediaSource for AnalysisFrames {
        fn info(&self) -> &MediaInfo {
            &self.info
        }

        fn video_frame(&self, request: FrameRequest) -> filmcraft_media::Result<Arc<filmcraft_frame::VideoFrame>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            let frame = self.info.frame_rate().frame_at(request.time).rem_euclid(16) as u8;
            Ok(Arc::new(filmcraft_frame::VideoFrame::rgba8(16, 8, [frame * 16, 96, 192, 255].repeat(16 * 8))))
        }

        fn audio(&self, _start: i64, frames: usize, rate: u32) -> filmcraft_media::Result<filmcraft_frame::AudioBuffer> {
            Ok(filmcraft_frame::AudioBuffer::silence(rate, 2, frames))
        }
    }

    struct ProxyServices(Vec<u8>);

    impl filmcraft_engine::Services for ProxyServices {
        fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
            if path == "analysis-proxy.png" { Ok(self.0.clone()) } else { Err(std::io::ErrorKind::NotFound.into()) }
        }

        fn write_file(&self, _path: &str, _data: &[u8]) -> std::io::Result<()> {
            Err(std::io::ErrorKind::Unsupported.into())
        }
    }

    struct AnalysisFixture {
        server: FrameServer,
        project: Arc<Project>,
        sequence: ItemId,
        reads: Arc<AtomicUsize>,
    }

    fn analysis_fixture(proxy: bool) -> AnalysisFixture {
        use filmcraft_project::{SequenceSettings, TrackKind};
        use filmcraft_time::TimeRange;
        use image::ImageEncoder;

        let pool = Arc::new(filmcraft_engine::media_pool::MediaPool::default());
        let mut project = Project::new("analysis namespace");
        let rate = FrameRate::FPS_25;
        let source = filmcraft_media::generators::GeneratorSource::new(Generator::BarsAndTone, 16, 8, rate, Tick::from_units(2, 1));
        let info = source.info().clone();
        let video = filmcraft_engine::demo::add_generator(&mut project, &pool, source, "analysis frames", Label::Iris, None);
        let reads = Arc::new(AtomicUsize::new(0));
        pool.insert(video, Arc::new(AnalysisFrames { info, reads: reads.clone() }));
        if proxy {
            let ItemKind::Media(media) = &mut project.item_mut(video).unwrap().kind else { panic!("generator is media") };
            media.proxy = Some(MediaRef::File { path: "analysis-proxy.png".into() });
            pool.set_use_proxies(true);
        }
        let sequence = project.new_sequence("stabilized", SequenceSettings { width: 16, height: 8, frame_rate: rate, ..Default::default() }, 1, 0, None);
        let mut clip = project.make_track_item(video, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, rate.tick_of(25)), rate).unwrap();
        let mut warp = filmcraft_project::find_effect("warp_stabilizer").unwrap().instance();
        warp.params.get_mut("framing").unwrap().value = filmcraft_project::ParamValue::Choice(0);
        clip.effects.insert(0, warp);
        for effect in &mut clip.effects {
            filmcraft_project::resolve_auto_points(effect, (16, 8), (16, 8));
        }
        project.sequence_mut(sequence).unwrap().video_tracks[0].items.push(clip);
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png).write_image(&[0, 255, 0, 255].repeat(16 * 8), 16, 8, image::ExtendedColorType::Rgba8).unwrap();
        let server = FrameServer::new(pool, Arc::new(ProxyServices(png)), Arc::new(filmcraft_engine::previews::PreviewStore::default()), 1);
        AnalysisFixture { server, project: Arc::new(project), sequence, reads }
    }

    fn analysis_picture(fixture: &AnalysisFixture, project: &Arc<Project>, frame: i64, revision: u64) -> Arc<Rgba> {
        let frame_key = FrameKey { target: Target::Sequence(fixture.sequence), frame, size: 1000, revision, draft: false };
        fixture.server.request_for(FrameView::Program, frame_key, FrameRate::FPS_25.tick_of(frame), 1.0, project, 0);
        assert!(wait_until(|| fixture.server.is_ready(&frame_key)), "tiny stabilized frame must complete");
        let picture = fixture.server.get(&frame_key).unwrap();
        assert_eq!((picture.w, picture.h), (16, 8));
        picture
    }

    #[test]
    fn frame_jobs_preserve_analysis_namespaces_across_projects_snapshots_and_proxy_modes() {
        filmcraft_render::vfx::clear_stabilizer_cache();
        let first = analysis_fixture(false);
        analysis_picture(&first, &first.project, 12, 0);
        let reads = first.reads.load(Ordering::SeqCst);
        assert!(reads >= 26, "first frame analyzes the complete one-second clip");
        analysis_picture(&first, &first.project, 13, 0);
        assert_eq!(first.reads.load(Ordering::SeqCst), reads + 1, "new jobs reuse analysis for an unchanged snapshot");

        let second = analysis_fixture(false);
        assert_eq!(first.sequence, second.sequence, "replacement projects deliberately reuse numeric IDs");
        analysis_picture(&second, &second.project, 12, 0);
        assert!(second.reads.load(Ordering::SeqCst) >= 26, "the timing provider must forward the independent pool namespace");
        let mut edited = (*second.project).clone();
        edited.sequence_mut(second.sequence).unwrap().video_tracks[0].items[0].frame_hold = Some(FrameRate::FPS_25.tick_of(5));
        let reads = second.reads.load(Ordering::SeqCst);
        analysis_picture(&second, &Arc::new(edited), 14, 1);
        assert!(second.reads.load(Ordering::SeqCst) >= reads + 26, "project edits need their own camera analysis");
        let reads = second.reads.load(Ordering::SeqCst);
        analysis_picture(&second, &second.project, 15, 2);
        assert_eq!(second.reads.load(Ordering::SeqCst), reads + 1, "retained snapshots keep their original analysis");

        let proxy = analysis_fixture(true);
        let proxy_picture = analysis_picture(&proxy, &proxy.project, 12, 0);
        assert_eq!(proxy.reads.load(Ordering::SeqCst), 0, "proxy warm-up does not decode originals");
        proxy.server.pool.set_use_proxies(false);
        let original_picture = analysis_picture(&proxy, &proxy.project, 13, 1);
        assert!(proxy.reads.load(Ordering::SeqCst) >= 26, "original playback cannot borrow proxy analysis");
        assert_ne!(proxy_picture.px, original_picture.px);
    }

    struct PlaybackSource {
        info: MediaInfo,
        calls: Arc<std::sync::Mutex<Vec<i64>>>,
        hold_frame: Option<i64>,
        entered: Sender<i64>,
        release: Arc<AtomicBool>,
    }

    impl MediaSource for PlaybackSource {
        fn info(&self) -> &MediaInfo {
            &self.info
        }

        fn video_frame(&self, req: FrameRequest) -> filmcraft_media::Result<Arc<filmcraft_frame::VideoFrame>> {
            let frame = FrameRate::FPS_24.frame_at(req.time);
            self.calls.lock().unwrap().push(frame);
            if self.hold_frame == Some(frame) {
                self.entered.send(frame).unwrap();
                while !self.release.load(Ordering::Acquire) && !filmcraft_media::cancel::cancelled() {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            if filmcraft_media::cancel::cancelled() {
                return Err(filmcraft_media::MediaError::Cancelled);
            }
            Ok(Arc::new(filmcraft_frame::VideoFrame::rgba8(16, 16, [frame as u8, 4, 8, 255].repeat(16 * 16))))
        }

        fn audio(&self, _start: i64, frames: usize, sample_rate: u32) -> filmcraft_media::Result<filmcraft_frame::AudioBuffer> {
            Ok(filmcraft_frame::AudioBuffer::silence(sample_rate, 2, frames))
        }
    }

    struct PlaybackFixture {
        server: FrameServer,
        project: Arc<Project>,
        key: FrameKey,
        calls: Arc<std::sync::Mutex<Vec<i64>>>,
        entered: std::sync::mpsc::Receiver<i64>,
        release: Arc<AtomicBool>,
        _release_on_failure: ReleaseOnDrop,
    }

    impl PlaybackFixture {
        fn schedule(&self, frame: i64, speed: f64, preroll: bool) {
            self.server.schedule_playback_for(FrameView::Program, FrameKey { frame, ..self.key }, FrameRate::FPS_24, 1.0, &self.project, speed, preroll);
        }

        fn idle(&self) -> bool {
            wait_until(|| {
                let queue = self.server.shared.queue.lock().unwrap();
                let running = self.server.shared.in_flight.lock().unwrap();
                queue.is_empty() && running.is_empty() && self.server.shared.stats.lock().unwrap().jobs == self.calls.lock().unwrap().len() as u64
            })
        }

        fn calls(&self) -> Vec<i64> {
            self.calls.lock().unwrap().clone()
        }
    }

    fn playback_fixture(budget: usize, hold_frame: Option<i64>) -> PlaybackFixture {
        let (entered_tx, entered) = channel();
        let release = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let generator = filmcraft_media::generators::GeneratorSource::new(Generator::BlackVideo, 16, 16, FrameRate::FPS_24, Tick::from_seconds_f64(2.0));
        let info = generator.info().clone();
        let source = Arc::new(PlaybackSource { info: info.clone(), calls: calls.clone(), hold_frame, entered: entered_tx, release: release.clone() });
        let mut project = Project::new("tiny playback preroll");
        let item = project.add_item(
            "counted playback",
            Label::Iris,
            ItemKind::Media(MediaClip {
                media: MediaRef::Generator(Generator::BlackVideo),
                info,
                interpret: Default::default(),
                mark_in: None,
                mark_out: None,
                markers: vec![],
                offline: false,
                proxy: None,
                identity: None,
            }),
            None,
        );
        let pool = Arc::new(filmcraft_engine::MediaPool::default());
        pool.insert(item, source);
        let server = FrameServer::new(pool, Arc::new(filmcraft_engine::FsServices), Arc::new(filmcraft_engine::previews::PreviewStore::default()), 1);
        // Scale the exact cache policy down instead of allocating oversized real-world images.
        server.shared.cache.lock().unwrap().budget = budget;
        PlaybackFixture {
            server,
            project: Arc::new(project),
            key: FrameKey { target: Target::Item(item), ..key(0, false) },
            calls,
            entered,
            release: release.clone(),
            _release_on_failure: ReleaseOnDrop(release),
        }
    }

    #[test]
    fn oversized_playback_starts_with_current_and_keeps_only_the_nearest_future() {
        let f = playback_fixture(16, Some(7));
        f.schedule(0, 1.0, true);
        assert!(f.idle(), "initial prefetch wave must retire without decoding distant frames");
        assert_eq!(f.calls(), [0, 1], "the old fourteen-frame wave decoded every future and replaced frame 1");
        assert!(f.server.preroll_ready(f.key, 1.0, 47), "the old six-frame condition cannot succeed with a one-future handoff");
        let picture = f.server.get(&f.key).expect("the current real source frame reaches the monitor");
        assert_eq!((picture.w, picture.h), (16, 16));
        assert!(f.server.is_ready(&FrameKey { frame: 1, ..f.key }));
        assert!(!f.server.is_ready(&FrameKey { frame: 2, ..f.key }));
        assert_eq!(f.server.cache_usage(), (0, 16));
        assert_eq!(f.server.working_usage(), (2, 2 * 16 * 16 * 4));
        drop(picture);

        f.schedule(1, 1.0, false);
        assert!(f.idle());
        assert_eq!(f.calls(), [0, 1, 2], "the ready nearest future is promoted instead of decoded again");
        assert!(!f.server.is_ready(&f.key), "superseded current handoff is released");
        assert!(f.server.is_ready(&FrameKey { frame: 1, ..f.key }));
        assert!(f.server.is_ready(&FrameKey { frame: 2, ..f.key }));
        assert_eq!(f.server.working_usage(), (2, 2 * 16 * 16 * 4));

        *f.server.shared.render_cost.lock().unwrap() = 0.2;
        let (lead, _) = super::super::playback_plan(0.2, 24.0, 1);
        assert_eq!(lead, 5);
        f.schedule(2, 1.0, false);
        assert_eq!(f.entered.recv_timeout(Duration::from_secs(2)).expect("paced future must enter the real source"), 7);
        assert!(f.server.is_ready(&FrameKey { frame: 2, ..f.key }), "a ready current is still promoted when pacing skips due jobs");
        assert!(!f.server.is_ready(&FrameKey { frame: 1, ..f.key }));
        for frame in 3..=6 {
            f.schedule(frame, 1.0, false);
            assert!(!f.server.is_ready(&FrameKey { frame, ..f.key }), "unrendered due frames remain skipped");
            let running = f.server.shared.in_flight.lock().unwrap();
            let future = running.iter().find(|job| job.key.frame == 7).expect("one paced render survives advancing paints");
            assert!(!future.cancel.load(Ordering::Relaxed), "an eligible future must not be cancelled for a new key+lead");
            assert!(future.demand.lock().unwrap().prefetch);
        }
        assert_eq!(f.calls(), [0, 1, 2, 7], "advancing paints do not start or reread another future");
        assert_eq!(f.server.working_usage(), (1, 16 * 16 * 4));
        f.release.store(true, Ordering::Release);
        assert!(f.idle(), "the preserved future finishes instead of being cancelled");
        assert!(f.server.is_ready(&FrameKey { frame: 7, ..f.key }));
        assert_eq!(f.server.working_usage(), (2, 2 * 16 * 16 * 4));

        *f.server.shared.render_cost.lock().unwrap() = 0.2;
        f.schedule(6, 1.0, false);
        assert!(f.idle());
        assert_eq!(f.calls(), [0, 1, 2, 7], "a ready eligible future also survives a new paint");
        f.schedule(7, 1.0, false);
        assert!(f.idle());
        assert_eq!(f.calls(), [0, 1, 2, 7, 7 + lead], "when due, the existing future is promoted without reread and the next lead begins");
        assert!(f.server.is_ready(&FrameKey { frame: 7, ..f.key }));
        assert!(!f.server.is_ready(&FrameKey { frame: 2, ..f.key }), "promotion releases the old displayed picture");
        assert_eq!(f.server.working_usage(), (2, 2 * 16 * 16 * 4));
        f.server.stop_prefetch();
        assert_eq!(f.server.working_usage(), (1, 16 * 16 * 4), "Stop releases the bounded future, keeping the displayed current");
    }

    #[test]
    fn oversized_playback_restart_retires_the_previous_wave_and_prefetches_useful_frames() {
        let f = playback_fixture(16, None);
        let old_current = FrameKey { frame: 47, ..f.key };
        let old_future = FrameKey { frame: 52, ..f.key };
        f.server.request_for(FrameView::Program, old_current, FrameRate::FPS_24.tick_of(47), 1.0, &f.project, 0);
        assert!(f.idle());
        *f.server.shared.render_cost.lock().unwrap() = 0.2;
        f.schedule(47, 1.0, false);
        assert!(f.idle());
        assert_eq!(f.calls(), [47, 52]);
        let obsolete = Arc::downgrade(&f.server.get(&old_future).expect("the previous wave has a ready paced future"));
        assert_eq!(f.server.working_usage(), (2, 2 * 16 * 16 * 4));

        // A 48-frame clip loops back to zero and enters preroll without stop_prefetch.
        f.schedule(0, 1.0, true);
        assert!(f.idle());
        assert_eq!(f.calls(), [47, 52, 0, 1], "restart creates its immediate useful future instead of keeping frame52");
        assert!(obsolete.upgrade().is_none(), "the old future's resources are released");
        assert!(!f.server.is_ready(&old_current) && !f.server.is_ready(&old_future));
        assert!(f.server.preroll_ready(f.key, 1.0, 47));
        assert!(f.server.is_ready(&FrameKey { frame: 1, ..f.key }));
        assert_eq!(f.server.cache_usage(), (0, 16));
        assert_eq!(f.server.working_usage(), (2, 2 * 16 * 16 * 4));
        f.schedule(0, 1.0, true);
        assert!(f.idle());
        assert_eq!(f.calls(), [47, 52, 0, 1], "repeated preroll paints keep their useful first future");

        *f.server.shared.render_cost.lock().unwrap() = 0.0;
        f.schedule(1, 1.0, false);
        assert!(f.idle());
        assert_eq!(f.calls(), [47, 52, 0, 1, 2], "useful playback resumes by promoting1 without decoding it again");
        assert!(f.server.is_ready(&FrameKey { frame: 1, ..f.key }));
        assert!(f.server.is_ready(&FrameKey { frame: 2, ..f.key }));
        assert!(!f.server.is_ready(&f.key));
        assert_eq!(f.server.working_usage(), (2, 2 * 16 * 16 * 4));
    }

    #[test]
    fn normal_budget_playback_still_waits_for_six_real_frames_and_prefetches_normally() {
        let f = playback_fixture(1 << 20, Some(5));
        f.schedule(0, 1.0, true);
        assert_eq!(f.entered.recv_timeout(Duration::from_secs(2)).expect("sixth frame must reach the source"), 5);
        assert!((0..5).all(|frame| f.server.is_ready(&FrameKey { frame, ..f.key })));
        assert!(!f.server.is_ready(&FrameKey { frame: 5, ..f.key }));
        assert!(!f.server.preroll_ready(f.key, 1.0, 47), "normal preroll still waits for its sixth frame");
        f.release.store(true, Ordering::Release);
        assert!(f.idle());
        assert!(f.server.preroll_ready(f.key, 1.0, 47));
        assert_eq!(f.calls(), (0..=14).collect::<Vec<_>>(), "normal fourteen-future scheduling is unchanged");
        assert_eq!(f.server.cache_usage(), (15 * 16 * 16 * 4, 1 << 20));
        assert_eq!(f.server.working_usage(), (0, 0));
    }

    struct HeldSource {
        info: MediaInfo,
        entered: Sender<()>,
        release: Arc<AtomicBool>,
        cancelled: Arc<AtomicBool>,
        calls: Arc<AtomicUsize>,
    }

    impl MediaSource for HeldSource {
        fn info(&self) -> &MediaInfo {
            &self.info
        }

        fn video_frame(&self, _req: FrameRequest) -> filmcraft_media::Result<Arc<filmcraft_frame::VideoFrame>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.entered.send(()).unwrap();
            // Hold the request even after observing cancellation, as a slow decoder/I/O might.
            while !self.release.load(Ordering::Acquire) {
                if filmcraft_media::cancel::cancelled() {
                    self.cancelled.store(true, Ordering::Release);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            if filmcraft_media::cancel::cancelled() {
                Err(filmcraft_media::MediaError::Cancelled)
            } else {
                Ok(Arc::new(filmcraft_frame::VideoFrame::rgba8(16, 16, vec![255; 16 * 16 * 4])))
            }
        }

        fn audio(&self, _start: i64, frames: usize, sample_rate: u32) -> filmcraft_media::Result<filmcraft_frame::AudioBuffer> {
            Ok(filmcraft_frame::AudioBuffer::silence(sample_rate, 2, frames))
        }
    }

    struct ReleaseOnDrop(Arc<AtomicBool>);

    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    struct HeldFixture {
        server: FrameServer,
        project: Arc<Project>,
        key: FrameKey,
        entered: std::sync::mpsc::Receiver<()>,
        release: Arc<AtomicBool>,
        calls: Arc<AtomicUsize>,
        _release_on_failure: ReleaseOnDrop,
    }

    fn held_fixture() -> HeldFixture {
        let (entered_tx, entered) = channel();
        let release = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let generator = filmcraft_media::generators::GeneratorSource::new(Generator::BlackVideo, 16, 16, FrameRate::FPS_24, Tick::from_seconds_f64(1.0));
        let info = generator.info().clone();
        let source = Arc::new(HeldSource {
            info: info.clone(),
            entered: entered_tx,
            release: release.clone(),
            cancelled: Arc::new(AtomicBool::new(false)),
            calls: calls.clone(),
        });
        let mut project = Project::new("held playback frame");
        let item = project.add_item(
            "held request",
            Label::Iris,
            ItemKind::Media(MediaClip {
                media: MediaRef::Generator(Generator::BlackVideo),
                info,
                interpret: Default::default(),
                mark_in: None,
                mark_out: None,
                markers: vec![],
                offline: false,
                proxy: None,
                identity: None,
            }),
            None,
        );
        let pool = Arc::new(filmcraft_engine::MediaPool::default());
        pool.insert(item, source);
        let server = FrameServer::new(pool, Arc::new(filmcraft_engine::FsServices), Arc::new(filmcraft_engine::previews::PreviewStore::default()), 1);
        // A tiny private budget exercises the handoff with a 16x16 frame and no large fixture.
        server.shared.cache.lock().unwrap().budget = 16;
        HeldFixture {
            server,
            project: Arc::new(project),
            key: FrameKey { target: Target::Item(item), ..key(0, false) },
            entered,
            release: release.clone(),
            calls,
            _release_on_failure: ReleaseOnDrop(release),
        }
    }

    #[test]
    fn hiding_views_discards_queue_and_cancels_active_work_without_joining() {
        let f = held_fixture();
        let visible = f.server.begin_view_pass();
        f.server.request_for(FrameView::Source, f.key, Tick::ZERO, 1.0, &f.project, 0);
        f.entered.recv_timeout(Duration::from_secs(2)).expect("visible source entered rendering");
        let queued_project = Arc::new((*f.project).clone());
        let queued_weak = Arc::downgrade(&queued_project);
        let next = FrameKey { frame: 1, ..f.key };
        f.server.request_for(FrameView::Comparison, next, FrameRate::FPS_24.tick_of(1), 1.0, &queued_project, 1);
        drop(queued_project);
        drop(visible);
        assert_eq!(f.server.queue_len(), 1);
        let empty = f.server.begin_view_pass();
        let (retired_tx, retired_rx) = channel();
        std::thread::spawn(move || {
            drop(empty);
            retired_tx.send(()).unwrap();
        });
        retired_rx.recv_timeout(Duration::from_secs(2)).expect("retirement returns while the source is held");
        assert_eq!(f.server.queue_len(), 0);
        assert!(queued_weak.upgrade().is_none());
        {
            let running = f.server.shared.in_flight.lock().unwrap();
            assert!(running.iter().all(|job| job.cancel.load(Ordering::Relaxed)));
        }
        f.release.store(true, Ordering::Release);
        assert!(wait_until(|| f.server.shared.in_flight.lock().unwrap().is_empty()));
        assert!(!f.server.is_ready(&f.key) && !f.server.is_ready(&next));
        assert_eq!(f.server.working_usage(), (0, 0));
        assert_eq!(f.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_running_prefetch_promotes_to_demand_and_survives_next_prefetch_and_stop() {
        let f = held_fixture();
        f.server.request_job(f.key, Tick::ZERO, 1.0, &f.project, 1, true, None, ViewId::Named(FrameView::Program));
        f.entered.recv_timeout(Duration::from_secs(2)).expect("prefetch entered source");
        f.server.request_for(FrameView::Program, f.key, Tick::ZERO, 1.0, &f.project, 0);
        // Stop while the source is held: the promoted job must not receive cancellation.
        f.server.stop_prefetch();
        f.release.store(true, Ordering::Release);
        assert!(wait_until(|| f.server.is_ready(&f.key)), "the running prefetch completes as demand");
        let shown = f.server.get(&f.key).unwrap();
        assert_eq!((shown.w, shown.h), (16, 16));
        assert_eq!(&shown.px[..4], &[255, 255, 255, 255]);
        let next = FrameKey { frame: 1, ..f.key };
        f.server.request_job(next, FrameRate::FPS_24.tick_of(1), 1.0, &f.project, 1, true, None, ViewId::Named(FrameView::Program));
        assert!(wait_until(|| f.server.is_ready(&next)));
        f.server.stop_prefetch();
        assert!(f.server.is_ready(&f.key), "a later prefetch and Stop preserve the demanded picture");
        assert!(!f.server.is_ready(&next), "Stop releases the next oversized prefetch");
        assert_eq!(f.server.working_usage(), (1, 16 * 16 * 4));
        assert_eq!(f.calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn playback_stale_frame_cleanup_preserves_other_views_of_the_same_sequence() {
        let f = held_fixture();
        f.server.request_for(FrameView::Comparison, f.key, Tick::ZERO, 1.0, &f.project, 1);
        f.entered.recv_timeout(Duration::from_secs(2)).expect("comparison entered source");
        f.server.request_for(FrameView::Program, f.key, Tick::ZERO, 1.0, &f.project, 0);
        f.server.retain_view_queue(ViewId::Named(FrameView::Program), |_| false);
        {
            let running = f.server.shared.in_flight.lock().unwrap();
            let active = running.iter().find(|j| j.key == f.key).unwrap();
            assert!(!active.cancel.load(Ordering::Relaxed));
            assert_eq!(active.demand.lock().unwrap().views, vec![ViewId::Named(FrameView::Comparison)]);
        }
        f.release.store(true, Ordering::Release);
        assert!(wait_until(|| f.server.is_ready(&f.key)), "playback moving ahead cannot cancel the parked comparison frame");
    }

    #[test]
    fn a_cached_replacement_detaches_old_running_demand_before_it_finishes() {
        let f = held_fixture();
        f.server.request_for(FrameView::Reference, f.key, Tick::ZERO, 1.0, &f.project, 3);
        f.entered.recv_timeout(Duration::from_secs(2)).expect("old reference entered source");
        let cached = FrameKey { target: Target::Sequence(ItemId(20)), frame: 1, size: 250, ..f.key };
        f.server.shared.cache.lock().unwrap().insert(cached, image(8));
        f.server.request_for(FrameView::Reference, cached, Tick::ZERO, 0.25, &f.project, 3);
        {
            let running = f.server.shared.in_flight.lock().unwrap();
            let active = running.iter().find(|j| j.key == f.key).unwrap();
            assert!(active.cancel.load(Ordering::Relaxed), "ready hits still retire the view's old running request");
        }
        f.release.store(true, Ordering::Release);
        assert!(wait_until(|| f.server.shared.in_flight.lock().unwrap().is_empty()));
        assert!(f.server.is_ready(&cached) && !f.server.is_ready(&f.key));
        assert_eq!(f.server.working_usage(), (0, 0));
    }

    #[test]
    fn a_dequeued_registered_prefetch_can_promote_before_running_after_stop() {
        let f = held_fixture();
        let mut queued = view_job(f.key, FrameView::Program, true, 1);
        queued.project = f.project.clone();
        let dequeued = {
            let mut queue = f.server.shared.queue.lock().unwrap();
            queue.push_back(queued);
            let dequeued = queue.pop_front().unwrap();
            assert!(super::super::register_job(&f.server.shared, &dequeued).is_some());
            dequeued
        };
        f.server.request_for(FrameView::Program, f.key, Tick::ZERO, 1.0, &f.project, 0);
        f.server.stop_prefetch();
        f.release.store(true, Ordering::Release);
        assert!(super::super::run_job(&f.server.shared, &dequeued, &f.server.pool, &f.server.services, &f.server.previews));
        assert!(f.server.is_ready(&f.key), "registration recheck uses promoted demand, not the initial prefetch flag");
        assert_eq!(f.calls.load(Ordering::Relaxed), 1);
        assert_eq!(f.server.working_usage(), (1, 16 * 16 * 4));
    }

    #[test]
    fn stopping_after_dequeue_before_registration_never_fetches_or_publishes() {
        let f = held_fixture();
        let mut dequeued = view_job(f.key, FrameView::Program, true, 1);
        dequeued.project = f.project.clone();
        dequeued.prefetch_epoch = f.server.shared.prefetch_epoch.load(Ordering::Acquire);
        // Model the worker's dequeue/register gap under the real queue lock, without a race.
        let dequeued = {
            let mut queue = f.server.shared.queue.lock().unwrap();
            queue.push_back(dequeued);
            queue.pop_front().unwrap()
        };
        f.server.stop_prefetch();
        assert!(super::super::run_job(&f.server.shared, &dequeued, &f.server.pool, &f.server.services, &f.server.previews));
        assert!(dequeued.cancel.load(Ordering::Relaxed));
        assert_eq!(f.calls.load(Ordering::Relaxed), 0, "stopped prefetch must not enter the source");
        assert!(!f.server.is_ready(&f.key));
        assert_eq!(f.server.working_usage(), (0, 0));
        assert!(f.server.shared.in_flight.lock().unwrap().is_empty());
    }

    #[test]
    fn trim_sides_do_not_cancel_each_other_and_shared_requests_keep_remaining_consumers() {
        let f = held_fixture();
        f.server.request_for(FrameView::TrimLeft, f.key, Tick::ZERO, 1.0, &f.project, 0);
        f.entered.recv_timeout(Duration::from_secs(2)).expect("left trim side entered source");
        f.server.request_for(FrameView::TrimRight, f.key, Tick::ZERO, 1.0, &f.project, 0);
        let left_next = FrameKey { frame: 1, ..f.key };
        f.server.request_for(FrameView::TrimLeft, left_next, FrameRate::FPS_24.tick_of(1), 1.0, &f.project, 0);
        {
            let running = f.server.shared.in_flight.lock().unwrap();
            let active = running.iter().find(|j| j.key == f.key).unwrap();
            assert!(!active.cancel.load(Ordering::Relaxed), "right side still wants this running request");
            assert_eq!(active.demand.lock().unwrap().views, vec![ViewId::Named(FrameView::TrimRight)]);
        }
        f.release.store(true, Ordering::Release);
        assert!(wait_until(|| f.server.is_ready(&f.key) && f.server.is_ready(&left_next)));
        assert_eq!(f.calls.load(Ordering::Relaxed), 2, "shared frame renders once and distinct trim frame separately");
        assert_eq!(f.server.working_usage(), (2, 2 * 16 * 16 * 4));
    }

    #[test]
    fn dropping_busy_server_cancels_work_discards_queue_and_does_not_join_on_ui() {
        let (entered_tx, entered_rx) = channel();
        let release = Arc::new(AtomicBool::new(false));
        let _release_on_failure = ReleaseOnDrop(release.clone());
        let cancelled = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let generator = filmcraft_media::generators::GeneratorSource::new(Generator::BlackVideo, 16, 16, FrameRate::FPS_24, Tick::from_seconds_f64(1.0));
        let info = generator.info().clone();
        let source =
            Arc::new(HeldSource { info: info.clone(), entered: entered_tx, release: release.clone(), cancelled: cancelled.clone(), calls: calls.clone() });
        let source_weak = Arc::downgrade(&source);
        let mut project = Project::new("frame shutdown");
        let item = project.add_item(
            "held request",
            Label::Iris,
            ItemKind::Media(MediaClip {
                media: MediaRef::Generator(Generator::BlackVideo),
                info,
                interpret: Default::default(),
                mark_in: None,
                mark_out: None,
                markers: vec![],
                offline: false,
                proxy: None,
                identity: None,
            }),
            None,
        );
        let project = Arc::new(project);
        let pool = Arc::new(filmcraft_engine::MediaPool::default());
        pool.insert(item, source);
        let server = FrameServer::new(pool, Arc::new(filmcraft_engine::FsServices), Arc::new(filmcraft_engine::previews::PreviewStore::default()), 1);
        let pool_weak = Arc::downgrade(&server.pool);
        let shared_weak = Arc::downgrade(&server.shared);
        let active_key = FrameKey { target: Target::Item(item), ..key(0, false) };
        server.request(active_key, Tick::ZERO, 1.0, &project, 0);
        entered_rx.recv_timeout(Duration::from_secs(2)).expect("worker entered source");
        let queued_project = Arc::new((*project).clone());
        let queued_weak = Arc::downgrade(&queued_project);
        server.request(FrameKey { frame: 1, ..active_key }, FrameRate::FPS_24.tick_of(1), 1.0, &queued_project, 1);
        drop(queued_project);
        let cached = image(32);
        let cached_weak = Arc::downgrade(&cached);
        server.shared.cache.lock().unwrap().insert(key(10, false), cached);
        let cached_plan = held_cached_plan(&server, key(11, true));
        let working = held_working_plan(&server, key(12, true), false);
        let (dropped_tx, dropped_rx) = channel();
        std::thread::spawn(move || {
            drop(server);
            dropped_tx.send(()).unwrap();
        });
        dropped_rx.recv_timeout(Duration::from_secs(2)).expect("Drop returns while the source is held");
        assert!(wait_until(|| cancelled.load(Ordering::Acquire)), "running sources receive cancellation");
        assert!(queued_weak.upgrade().is_none(), "queued project released before the running request finishes");
        assert!(cached_weak.upgrade().is_none(), "cached frames released before the running request finishes");
        assert!(cached_plan.upgrade().is_none() && working.upgrade().is_none(), "GPU cache and handoff released before the running request finishes");
        assert!(source_weak.upgrade().is_some(), "the held request has not exited yet");
        release.store(true, Ordering::Release);
        assert!(wait_until(|| pool_weak.upgrade().is_none() && shared_weak.upgrade().is_none() && source_weak.upgrade().is_none()));
        assert_eq!(calls.load(Ordering::Relaxed), 1, "shutdown never begins the queued request");
    }
}
