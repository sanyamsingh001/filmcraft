# UNTESTED FilmCraft source recovery — 2026-10-19

These separate recovery branches preserve surviving source variants before an authorized workspace reset. They are not feature pull requests, a consolidated candidate, or a validation claim. New recovery commits use the requested contributor identity; original commit authors and history are unchanged.

Working-source snapshots exclude credentials, private logs/capture material, build outputs, and fourteen generated H.264 oracle files below crates/h264enc/target. Existing historical commits are referenced unchanged. Product source, tests, documentation, and existing source assets remain in the individual snapshots.

Some newer private Git metadata was unavailable, so their physical source snapshots are parentless. Two later unmaterialized proposals were not recoverable: the TTML main6c adaptation and the Preview typed-Events API adapter. Older TTML and Preview implementations are preserved below.

## Working-source variants

| Variant | Exact recovery commit | Source files |
| --- | --- | --- |
| canonical | [254611833a7e8b75dd33daca33f832735195d285](https://github.com/sanyamsingh001/filmcraft/tree/254611833a7e8b75dd33daca33f832735195d285) | 1116 |
| desktop-mime-generator-main6c-20261019 | [56532eb38f1b99ec542544d88fe3e31df975b22d](https://github.com/sanyamsingh001/filmcraft/tree/56532eb38f1b99ec542544d88fe3e31df975b22d) | 1361 |
| events-inherited-terminal-cause-after-main6c-20261019 | [28eb422dd797eb318d9c48c3af426fe5a12948bd](https://github.com/sanyamsingh001/filmcraft/tree/28eb422dd797eb318d9c48c3af426fe5a12948bd) | 1361 |
| events-inherited-terminal-cause-before-main6c-20261019 | [daa0e2beada10425b6217b9c05ea2a11b649c85e](https://github.com/sanyamsingh001/filmcraft/tree/daa0e2beada10425b6217b9c05ea2a11b649c85e) | 1361 |
| progress-typed-events-main6c-20261019 | [cb01f7a5b4466fa9ae7edba8e3fbea5558713421](https://github.com/sanyamsingh001/filmcraft/tree/cb01f7a5b4466fa9ae7edba8e3fbea5558713421) | 1361 |
| tooltip-on-typed-progress-main6c-20261019 | [217c9661b45bf156ef02278048fae8fa51317e34](https://github.com/sanyamsingh001/filmcraft/tree/217c9661b45bf156ef02278048fae8fa51317e34) | 1361 |
| video-preview-terminal-cause-after-on-events-oct18 | [f2a2825a40c497f67fe4739d8190f28b29d1d268](https://github.com/sanyamsingh001/filmcraft/tree/f2a2825a40c497f67fe4739d8190f28b29d1d268) | 1351 |
| progress-terminal-cause-main6c-20261019 | [24f845575b8cb998c096d46fdff41c16b8d3e882](https://github.com/sanyamsingh001/filmcraft/tree/24f845575b8cb998c096d46fdff41c16b8d3e882) | 1361 |
| events-terminal-cause-on-progress-oct15 | [1cf364026c7e50b50f31df53c9bddcf8325e23fe](https://github.com/sanyamsingh001/filmcraft/tree/1cf364026c7e50b50f31df53c9bddcf8325e23fe) | 1351 |
| progress-completed-row-tooltip-oct15 | [9c9352be133c92241e3af4744daf4d79eede7035](https://github.com/sanyamsingh001/filmcraft/tree/9c9352be133c92241e3af4744daf4d79eede7035) | 1351 |
| audio-stream-essence | [5d92e686149a1328e05f72480839273819c45e32](https://github.com/sanyamsingh001/filmcraft/tree/5d92e686149a1328e05f72480839273819c45e32) | 1102 |
| caption-sidecar-precision-oct13 | [fe46780e745f1a15cebe94ddb465a36c557a9f22](https://github.com/sanyamsingh001/filmcraft/tree/fe46780e745f1a15cebe94ddb465a36c557a9f22) | 1351 |
| caption-sidecar-range-oct12 | [6b0775aabb85556f3f04d4c06ebab02de000badf](https://github.com/sanyamsingh001/filmcraft/tree/6b0775aabb85556f3f04d4c06ebab02de000badf) | 1328 |
| caption-txt-on-precision-oct14 | [b9219fdb05b5979f8f407c28446cfe6094f5ecfb](https://github.com/sanyamsingh001/filmcraft/tree/b9219fdb05b5979f8f407c28446cfe6094f5ecfb) | 1352 |
| control-input | [c2ffcb31d8f5f67dbfb2d1474571ee3eee4c6b9c](https://github.com/sanyamsingh001/filmcraft/tree/c2ffcb31d8f5f67dbfb2d1474571ee3eee4c6b9c) | 1052 |
| crop | [55cb4005f70a2738705a7c8708d55fed1e2b5611](https://github.com/sanyamsingh001/filmcraft/tree/55cb4005f70a2738705a7c8708d55fed1e2b5611) | 1051 |
| dock-layout | [502c362e0c706bf70adf36be839c2571b95d3632](https://github.com/sanyamsingh001/filmcraft/tree/502c362e0c706bf70adf36be839c2571b95d3632) | 1051 |
| export-job-tracking-docs-oct14 | [e41f8cce091f1b5bc485a8e50d354d7d7ffb122b](https://github.com/sanyamsingh001/filmcraft/tree/e41f8cce091f1b5bc485a8e50d354d7d7ffb122b) | 1350 |
| export-target-timebase-after-main6c-20261019 | [aca40309faa32e6f5f73cf09362d0ca53af6312b](https://github.com/sanyamsingh001/filmcraft/tree/aca40309faa32e6f5f73cf09362d0ca53af6312b) | 1361 |
| export-target-timebase-before-main6c-20261019 | [fdef8c5b33a7275efd42f499c65bca62feee5958](https://github.com/sanyamsingh001/filmcraft/tree/fdef8c5b33a7275efd42f499c65bca62feee5958) | 1361 |
| export-target-timebase-on-precision-oct14 | [510f8379ea31c8e186b9b2cd00a63bd58400a2c6](https://github.com/sanyamsingh001/filmcraft/tree/510f8379ea31c8e186b9b2cd00a63bd58400a2c6) | 1351 |
| frames | [ac23d50c7797b579aa3d36406fbaafdba4acfdd7](https://github.com/sanyamsingh001/filmcraft/tree/ac23d50c7797b579aa3d36406fbaafdba4acfdd7) | 1051 |
| fullscreen | [4fa83afa55360174551c5719f8685066f4a1623c](https://github.com/sanyamsingh001/filmcraft/tree/4fa83afa55360174551c5719f8685066f4a1623c) | 1051 |
| gop-maintenance | [fb0cd8e25421fcd82df5012803f7c99fbdba8bbe](https://github.com/sanyamsingh001/filmcraft/tree/fb0cd8e25421fcd82df5012803f7c99fbdba8bbe) | 1052 |
| graphics-layer-selection-main6c-20261019 | [be7d89eae6c22227c138502a194d6de0849d98f0](https://github.com/sanyamsingh001/filmcraft/tree/be7d89eae6c22227c138502a194d6de0849d98f0) | 1361 |
| graphics-layer-selection-oct14 | [51ae6ea32ac9cdbb7813f452d72fae96ded9ea0d](https://github.com/sanyamsingh001/filmcraft/tree/51ae6ea32ac9cdbb7813f452d72fae96ded9ea0d) | 1351 |
| header-menu-overlap-after-main09-20261017 | [43affcff3c57ec02618cd0182194b0f6c8cb5c36](https://github.com/sanyamsingh001/filmcraft/tree/43affcff3c57ec02618cd0182194b0f6c8cb5c36) | 1355 |
| header-menu-overlap-after-main480-20261018 | [85141b96bfbd7e79146a5a408beb10ad65ff2def](https://github.com/sanyamsingh001/filmcraft/tree/85141b96bfbd7e79146a5a408beb10ad65ff2def) | 1360 |
| header-menu-overlap-before-main09-20261017 | [ab03144c10c94f8e20fdd536ca630ef3a87291de](https://github.com/sanyamsingh001/filmcraft/tree/ab03144c10c94f8e20fdd536ca630ef3a87291de) | 1355 |
| header-menu-overlap-before-main480-20261018 | [59f4d2abe2113626da9165780143b1395858d2ec](https://github.com/sanyamsingh001/filmcraft/tree/59f4d2abe2113626da9165780143b1395858d2ec) | 1360 |
| held-effect-time | [ce2339c82584f7d6578bf57af87d86b102b33057](https://github.com/sanyamsingh001/filmcraft/tree/ce2339c82584f7d6578bf57af87d86b102b33057) | 1052 |
| ingest-collisions-oct12 | [aa0699dda3556dfbf5ccf75280fd7d4e766fe273](https://github.com/sanyamsingh001/filmcraft/tree/aa0699dda3556dfbf5ccf75280fd7d4e766fe273) | 1328 |
| inherited-job-terminal-cause-before-on-events-oct19 | [6ca15f59873ae03299304a3463592dbe237c8284](https://github.com/sanyamsingh001/filmcraft/tree/6ca15f59873ae03299304a3463592dbe237c8284) | 1351 |
| inspectors-selection-oct12 | [325505cae424bc6796a1d21b99c2cd2da3190ded](https://github.com/sanyamsingh001/filmcraft/tree/325505cae424bc6796a1d21b99c2cd2da3190ded) | 1328 |
| install | [ebdd0bd23a7de1cdb574f05d4f8c37551561018e](https://github.com/sanyamsingh001/filmcraft/tree/ebdd0bd23a7de1cdb574f05d4f8c37551561018e) | 1052 |
| integration | [54527ee6742b939dec82e5fa0a320cc2fecda354](https://github.com/sanyamsingh001/filmcraft/tree/54527ee6742b939dec82e5fa0a320cc2fecda354) | 1084 |
| interchange-export-feedback-oct12 | [e6d8c40977ac82c4638294a3412113f3f4b880d2](https://github.com/sanyamsingh001/filmcraft/tree/e6d8c40977ac82c4638294a3412113f3f4b880d2) | 1328 |
| interchange-export-feedback-oct13 | [5da7afc09a88b8474e21c085dad69d36a94b8390](https://github.com/sanyamsingh001/filmcraft/tree/5da7afc09a88b8474e21c085dad69d36a94b8390) | 1350 |
| interchange-notes-catalogs-581-20261019 | [389630d44dfce68fa5b170b6f5a4a61812ab5b49](https://github.com/sanyamsingh001/filmcraft/tree/389630d44dfce68fa5b170b6f5a4a61812ab5b49) | 1351 |
| interchange-repeat-status-oct14 | [fd8cf5fe51d60181e63670aad453a32ae61fcef1](https://github.com/sanyamsingh001/filmcraft/tree/fd8cf5fe51d60181e63670aad453a32ae61fcef1) | 1351 |
| interchange-timer-after-main6c-20261019 | [c19697028ecfec834bb71748d21ebb7094442b28](https://github.com/sanyamsingh001/filmcraft/tree/c19697028ecfec834bb71748d21ebb7094442b28) | 1361 |
| interchange-timer-before-main6c-20261019 | [0823f63654c5bd35d0ba1158455c52d7fe451c13](https://github.com/sanyamsingh001/filmcraft/tree/0823f63654c5bd35d0ba1158455c52d7fe451c13) | 1361 |
| lens-blur-sampling-oct14 | [4b176127ae7314a92affac37aa96c50bf3bc46f8](https://github.com/sanyamsingh001/filmcraft/tree/4b176127ae7314a92affac37aa96c50bf3bc46f8) | 1350 |
| lifecycle | [b1cc12987bbe335e0f66359a5ebd7a99f3e1564e](https://github.com/sanyamsingh001/filmcraft/tree/b1cc12987bbe335e0f66359a5ebd7a99f3e1564e) | 1051 |
| linked-audio | [7660e1d2d76e66ae6ec76c7c9e1d475a4a7f9f9d](https://github.com/sanyamsingh001/filmcraft/tree/7660e1d2d76e66ae6ec76c7c9e1d475a4a7f9f9d) | 1051 |
| multi-audio-followup | [d4bcbaec38607b8b73cad6b8a7e3225d496a9365](https://github.com/sanyamsingh001/filmcraft/tree/d4bcbaec38607b8b73cad6b8a7e3225d496a9365) | 1053 |
| named-targets | [598ad627fcab8c9565f2084f216c2cc4ed6ebd62](https://github.com/sanyamsingh001/filmcraft/tree/598ad627fcab8c9565f2084f216c2cc4ed6ebd62) | 1050 |
| parity-oct12-main | [c092e3da1a4d175044afc893dd7bf44bc4ba7906](https://github.com/sanyamsingh001/filmcraft/tree/c092e3da1a4d175044afc893dd7bf44bc4ba7906) | 1328 |
| pickers | [c8fe7d6a92b33e62faf13db731b7ed73366a25c9](https://github.com/sanyamsingh001/filmcraft/tree/c8fe7d6a92b33e62faf13db731b7ed73366a25c9) | 1053 |
| point | [8667f93ef703e921da75082d1318c1cb19739d36](https://github.com/sanyamsingh001/filmcraft/tree/8667f93ef703e921da75082d1318c1cb19739d36) | 1052 |
| progress-export-error-oct14 | [ebdd24ddace743f369955f02951a69a057eda6a3](https://github.com/sanyamsingh001/filmcraft/tree/ebdd24ddace743f369955f02951a69a057eda6a3) | 1351 |
| proxy-pending-output-collision-oct14 | [59f5973d74f7c8acfb0100635f71cc67c2b7416d](https://github.com/sanyamsingh001/filmcraft/tree/59f5973d74f7c8acfb0100635f71cc67c2b7416d) | 1350 |
| proxy-pending-output-currentmain-c2f-baseline-20261016-v1 | [0f01bb0eb46bfbb0d9de48968190b65d78aa4e8b](https://github.com/sanyamsingh001/filmcraft/tree/0f01bb0eb46bfbb0d9de48968190b65d78aa4e8b) | 1355 |
| readers | [1585c95f277d4402839a5350668d0871ed9a12cf](https://github.com/sanyamsingh001/filmcraft/tree/1585c95f277d4402839a5350668d0871ed9a12cf) | 1052 |
| scalar-graph-press-origin | [896cc2569d2d87e1605f817adcc798cc41bd8b88](https://github.com/sanyamsingh001/filmcraft/tree/896cc2569d2d87e1605f817adcc798cc41bd8b88) | 1116 |
| shortcut-defaults | [effb70f763d67d9eca6596d4d3ec237927b5e1b0](https://github.com/sanyamsingh001/filmcraft/tree/effb70f763d67d9eca6596d4d3ec237927b5e1b0) | 1050 |
| source | [ff2663d3da1803dd87e65e639bdde37fcb4ede04](https://github.com/sanyamsingh001/filmcraft/tree/ff2663d3da1803dd87e65e639bdde37fcb4ede04) | 1052 |
| source-export | [12dc014c7e2652af63c8937f4d7cec8d73e17557](https://github.com/sanyamsingh001/filmcraft/tree/12dc014c7e2652af63c8937f4d7cec8d73e17557) | 1052 |
| speech-pre-request-cancellation-currentmain-c2f-20261016-v1 | [0227a90297a042d617e59d254747dbed8ed96d7a](https://github.com/sanyamsingh001/filmcraft/tree/0227a90297a042d617e59d254747dbed8ed96d7a) | 1355 |
| speech-pre-request-cancellation-oct14 | [c158ef5ac3103af7333e1a55b9b03ba051482c64](https://github.com/sanyamsingh001/filmcraft/tree/c158ef5ac3103af7333e1a55b9b03ba051482c64) | 1350 |
| speed-dialog | [541337c3cd41e4fe67fe2bc59ef6785c68b3dd71](https://github.com/sanyamsingh001/filmcraft/tree/541337c3cd41e4fe67fe2bc59ef6785c68b3dd71) | 1050 |
| stabilizer-cancellation-oct14 | [26b1712eff1efc344887e64402ddba59ec67d833](https://github.com/sanyamsingh001/filmcraft/tree/26b1712eff1efc344887e64402ddba59ec67d833) | 1350 |
| time | [78f6df744041fb1d7c0262d3a47eee7b1b4f4939](https://github.com/sanyamsingh001/filmcraft/tree/78f6df744041fb1d7c0262d3a47eee7b1b4f4939) | 1050 |
| transition | [bf4bef8635c5b2834eff74b640afd0e85ffbcdc6](https://github.com/sanyamsingh001/filmcraft/tree/bf4bef8635c5b2834eff74b640afd0e85ffbcdc6) | 1052 |
| ttml-cue-ids-oct13 | [500b3e3765e924e3206beef4f6c0c0331ee26259](https://github.com/sanyamsingh001/filmcraft/tree/500b3e3765e924e3206beef4f6c0c0331ee26259) | 1351 |
| ttml-cue-ids-on-precision-oct13 | [9f0a684407a4339d1feec544568b5b69bb242600](https://github.com/sanyamsingh001/filmcraft/tree/9f0a684407a4339d1feec544568b5b69bb242600) | 1352 |
| ttml-speaker-metadata-oct12 | [070c2f26cf8d51fc2fe85b979442e9741498253d](https://github.com/sanyamsingh001/filmcraft/tree/070c2f26cf8d51fc2fe85b979442e9741498253d) | 1329 |
| video-preview-terminal-cause-before-on-events-oct16 | [e24caca18cec3652d92d029a145ec25b3fd81bdd](https://github.com/sanyamsingh001/filmcraft/tree/e24caca18cec3652d92d029a145ec25b3fd81bdd) | 1351 |
| wav | [906245e4c17812d55bd806fc29ff4810934b3d8d](https://github.com/sanyamsingh001/filmcraft/tree/906245e4c17812d55bd806fc29ff4810934b3d8d) | 1052 |

## Staged source and local stash

These retain source states that were separate from the working directories. The stash-source recovery commit retains the original stash commit f7a1e30e9962dae99fb30fcbca815e8aa0a7db3b and its parents in ancestry.

| State | Exact recovery commit |
| --- | --- |
| index-integration | [751d8fb9639288944154b4f40a8e6911a9dbabdf](https://github.com/sanyamsingh001/filmcraft/tree/751d8fb9639288944154b4f40a8e6911a9dbabdf) |
| index-time | [1a86a4a6f61086f60a4711e158f91714bdeaec99](https://github.com/sanyamsingh001/filmcraft/tree/1a86a4a6f61086f60a4711e158f91714bdeaec99) |
| index-transition | [8e6f3aeb316e22fd1815f3ca6f2f7db412ff0b8a](https://github.com/sanyamsingh001/filmcraft/tree/8e6f3aeb316e22fd1815f3ca6f2f7db412ff0b8a) |
| stash-source | [7316c5e0977986efc00232e7ab1fca39a65648ef](https://github.com/sanyamsingh001/filmcraft/tree/7316c5e0977986efc00232e7ab1fca39a65648ef) |

## Preserved original commit histories

These exact original commits were independently present on the fork network and now have dedicated UNTESTED recovery references.

| Recovery branch | Original commit | Original local references |
| --- | --- | --- |
| recovery/untested-20261019/history-0541d4d182d0 | [0541d4d182d0bb755a2633ef66824cdadb280c09](https://github.com/sanyamsingh001/filmcraft/commit/0541d4d182d0bb755a2633ef66824cdadb280c09) | refs/heads/codex/interchange-export-feedback-oct12, worktree/interchange-export-feedback-oct12, worktree/parity-oct12-main |
| recovery/untested-20261019/history-09aaf9bf15d5 | [09aaf9bf15d510eda1c413c0ccabb1d7e08b41c5](https://github.com/sanyamsingh001/filmcraft/commit/09aaf9bf15d510eda1c413c0ccabb1d7e08b41c5) | worktree/header-menu-overlap-after-main09-20261017, worktree/header-menu-overlap-before-main09-20261017 |
| recovery/untested-20261019/history-0e0f0485da1a | [0e0f0485da1a9cf9feba6b60f04cecd58219d22a](https://github.com/sanyamsingh001/filmcraft/commit/0e0f0485da1a9cf9feba6b60f04cecd58219d22a) | refs/heads/codex/linux-validation-baseline, refs/heads/codex/scalar-graph-press-origin |
| recovery/untested-20261019/history-10f408ad811c | [10f408ad811c501fa10a8d9e0c95fffd9889fd89](https://github.com/sanyamsingh001/filmcraft/commit/10f408ad811c501fa10a8d9e0c95fffd9889fd89) | refs/heads/codex/normal-video-preview-terminal-cause-on-events-oct18, worktree/video-preview-terminal-cause-after-on-events-oct18 |
| recovery/untested-20261019/history-13c89bf9df04 | [13c89bf9df04e468bcc7be65a7fb56cdb4e26966](https://github.com/sanyamsingh001/filmcraft/commit/13c89bf9df04e468bcc7be65a7fb56cdb4e26966) | refs/heads/codex/caption-sidecar-range-oct12, worktree/caption-sidecar-range-oct12 |
| recovery/untested-20261019/history-2343fc8fa37e | [2343fc8fa37e2afa6e4bca29cddf577987472e06](https://github.com/sanyamsingh001/filmcraft/commit/2343fc8fa37e2afa6e4bca29cddf577987472e06) | refs/heads/codex/ttml-speaker-metadata-oct12, worktree/ttml-speaker-metadata-oct12 |
| recovery/untested-20261019/history-23f79890fd20 | [23f79890fd205cbabdbc296e5e120f38234db589](https://github.com/sanyamsingh001/filmcraft/commit/23f79890fd205cbabdbc296e5e120f38234db589) | refs/heads/codex/scalar-graph-current-main, worktree/scalar-graph-press-origin |
| recovery/untested-20261019/history-353fa96eeb4c | [353fa96eeb4c57de63602258a6d4ed221276cee5](https://github.com/sanyamsingh001/filmcraft/commit/353fa96eeb4c57de63602258a6d4ed221276cee5) | refs/heads/codex/ttml-cue-ids-on-precision-oct13, worktree/ttml-cue-ids-on-precision-oct13 |
| recovery/untested-20261019/history-51d35c3277d2 | [51d35c3277d2eab8d0266276437c64e5377fbdbc](https://github.com/sanyamsingh001/filmcraft/commit/51d35c3277d2eab8d0266276437c64e5377fbdbc) | refs/heads/codex/linux-dev-desktop-mime-oct13 |
| recovery/untested-20261019/history-523185244336 | [5231852443363f001c3f6b396dd9b1e6461ae2be](https://github.com/sanyamsingh001/filmcraft/commit/5231852443363f001c3f6b396dd9b1e6461ae2be) | refs/heads/codex/control-input-20261009, refs/heads/codex/crop-20261009, refs/heads/codex/dock-layout-20261009, refs/heads/codex/frames-20261009, refs/heads/codex/fullscreen-20261009, refs/heads/codex/gop-maintenance-20261009, refs/heads/codex/held-effect-time-20261009, refs/heads/codex/issues-and-compat-20261009, refs/heads/codex/lifecycle-20261009, refs/heads/codex/linked-audio-effects-20261009, refs/heads/codex/linux-install-20261009, refs/heads/codex/linux-pickers-20261009, refs/heads/codex/linux-readers-20261009, refs/heads/codex/named-targets-20261009, refs/heads/codex/point-graph-20261009, refs/heads/codex/shortcut-defaults-20261009, refs/heads/codex/source-export-20261009, refs/heads/codex/source-monitor-20261009, refs/heads/codex/speed-dialog-20261009, refs/heads/codex/time-20261009, refs/heads/codex/transition-20261009, refs/heads/codex/verified-integration-20261009, refs/heads/codex/wav-20261009, worktree/control-input, worktree/crop, worktree/dock-layout, worktree/frames, worktree/fullscreen, worktree/gop-maintenance, worktree/held-effect-time, worktree/install, worktree/integration, worktree/lifecycle, worktree/linked-audio, worktree/named-targets, worktree/pickers, worktree/point, worktree/readers, worktree/shortcut-defaults, worktree/source, worktree/source-export, worktree/speed-dialog, worktree/time, worktree/transition, worktree/wav |
| recovery/untested-20261019/history-5811820b0911 | [5811820b091192984af7ad26056df493be153bfb](https://github.com/sanyamsingh001/filmcraft/commit/5811820b091192984af7ad26056df493be153bfb) | refs/heads/codex/interchange-export-feedback-on-precision-oct13, worktree/caption-sidecar-precision-oct13 |
| recovery/untested-20261019/history-61238bf8fc12 | [61238bf8fc129077fe2b7982550303be49f55a93](https://github.com/sanyamsingh001/filmcraft/commit/61238bf8fc129077fe2b7982550303be49f55a93) | refs/heads/main |
| recovery/untested-20261019/history-6714661b12ea | [6714661b12ea18cd02cbc6a09a3acd78bcc853f5](https://github.com/sanyamsingh001/filmcraft/commit/6714661b12ea18cd02cbc6a09a3acd78bcc853f5) | refs/heads/codex/events-terminal-cause-on-progress-oct15, worktree/inherited-job-terminal-cause-before-on-events-oct19, worktree/video-preview-terminal-cause-before-on-events-oct16 |
| recovery/untested-20261019/history-851cf2fe52de | [851cf2fe52ded83e2eba928d095a66bdcfb6aa35](https://github.com/sanyamsingh001/filmcraft/commit/851cf2fe52ded83e2eba928d095a66bdcfb6aa35) | refs/heads/codex/graphics-layer-selection-oct14 |
| recovery/untested-20261019/history-892003f3d4aa | [892003f3d4aa7617f7e708b5f1c35b502c846f0c](https://github.com/sanyamsingh001/filmcraft/commit/892003f3d4aa7617f7e708b5f1c35b502c846f0c) | refs/heads/codex/gpu-effect-source, refs/heads/codex/time-frame-boundaries |
| recovery/untested-20261019/history-8939b15501cf | [8939b15501cfbd167b4dded307e1cd5400bfacb9](https://github.com/sanyamsingh001/filmcraft/commit/8939b15501cfbd167b4dded307e1cd5400bfacb9) | refs/heads/codex/caption-sidecar-precision-on-mime-oct13, refs/heads/codex/caption-txt-on-precision-oct14, refs/heads/codex/progress-export-error-oct14, worktree/caption-txt-on-precision-oct14, worktree/progress-export-error-oct14 |
| recovery/untested-20261019/history-9b2621398373 | [9b26213983734100cf85d6d951c995e8f3dd03c6](https://github.com/sanyamsingh001/filmcraft/commit/9b26213983734100cf85d6d951c995e8f3dd03c6) | refs/heads/codex/progress-completed-row-tooltip-oct15, refs/heads/codex/progress-terminal-cause-on-precision-oct14, worktree/graphics-layer-selection-oct14, worktree/progress-completed-row-tooltip-oct15 |
| recovery/untested-20261019/history-a3ad7a8ce742 | [a3ad7a8ce742e208a3199519e083a59c2ca68927](https://github.com/sanyamsingh001/filmcraft/commit/a3ad7a8ce742e208a3199519e083a59c2ca68927) | refs/heads/codex/time-current-main, worktree/canonical |
| recovery/untested-20261019/history-a70a4dbfb590 | [a70a4dbfb5903dcbac781610eb57d3f011ea0366](https://github.com/sanyamsingh001/filmcraft/commit/a70a4dbfb5903dcbac781610eb57d3f011ea0366) | refs/heads/codex/progress-completed-tooltip-on-events-oct16, worktree/events-terminal-cause-on-progress-oct15 |
| recovery/untested-20261019/history-c1edf9e61620 | [c1edf9e61620e0583dabe85bc8f87dc759ae4a53](https://github.com/sanyamsingh001/filmcraft/commit/c1edf9e61620e0583dabe85bc8f87dc759ae4a53) | refs/heads/codex/export-target-timebase-on-precision-oct14, worktree/export-target-timebase-on-precision-oct14 |
| recovery/untested-20261019/history-c2f65871e998 | [c2f65871e998cca4700a74dbcf9348cab51fb06b](https://github.com/sanyamsingh001/filmcraft/commit/c2f65871e998cca4700a74dbcf9348cab51fb06b) | worktree/proxy-pending-output-currentmain-c2f-baseline-20261016-v1, worktree/speech-pre-request-cancellation-currentmain-c2f-20261016-v1 |
| recovery/untested-20261019/history-cee1feb185e6 | [cee1feb185e675a108f78067f71ad86a640ace3d](https://github.com/sanyamsingh001/filmcraft/commit/cee1feb185e675a108f78067f71ad86a640ace3d) | refs/heads/codex/caption-sidecar-precision-oct13, refs/heads/codex/export-job-tracking-docs-oct14, refs/heads/codex/interchange-export-feedback-oct13, refs/heads/codex/lens-blur-sampling-oct14, refs/heads/codex/proxy-pending-output-collision-oct14, refs/heads/codex/speech-pre-request-cancellation-oct14, refs/heads/codex/stabilizer-cancellation-oct14, refs/heads/codex/ttml-cue-ids-oct13, worktree/export-job-tracking-docs-oct14, worktree/interchange-export-feedback-oct13, worktree/lens-blur-sampling-oct14, worktree/proxy-pending-output-collision-oct14, worktree/speech-pre-request-cancellation-oct14, worktree/stabilizer-cancellation-oct14, worktree/ttml-cue-ids-oct13 |
| recovery/untested-20261019/history-d91a7b312291 | [d91a7b3122913167d1050a2fd4bf987a5388bf27](https://github.com/sanyamsingh001/filmcraft/commit/d91a7b3122913167d1050a2fd4bf987a5388bf27) | refs/heads/codex/inspectors-selection-oct12, worktree/inspectors-selection-oct12 |
| recovery/untested-20261019/history-dd121aeb9c52 | [dd121aeb9c5203353f889423f115b8bd8059f471](https://github.com/sanyamsingh001/filmcraft/commit/dd121aeb9c5203353f889423f115b8bd8059f471) | refs/heads/codex/audio-stream-essence, worktree/audio-stream-essence |
| recovery/untested-20261019/history-f146d7ba9fed | [f146d7ba9fedb75666c99f03b3bd8e3911eeb393](https://github.com/sanyamsingh001/filmcraft/commit/f146d7ba9fedb75666c99f03b3bd8e3911eeb393) | refs/heads/codex/audio-peer361-linux-baseline, worktree/multi-audio-followup |
| recovery/untested-20261019/history-f4acdb445b44 | [f4acdb445b446175f9084f168a6b031a40d8af4e](https://github.com/sanyamsingh001/filmcraft/commit/f4acdb445b446175f9084f168a6b031a40d8af4e) | refs/heads/codex/interchange-repeat-status-oct14, worktree/interchange-repeat-status-oct14 |
| recovery/untested-20261019/history-fa7c6ff8abd0 | [fa7c6ff8abd0ce06e0b4860705cf0b9e6d41001d](https://github.com/sanyamsingh001/filmcraft/commit/fa7c6ff8abd0ce06e0b4860705cf0b9e6d41001d) | refs/heads/codex/ingest-collisions-oct12, worktree/ingest-collisions-oct12 |
