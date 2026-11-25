//! Where the models run, in words: the device chip's state and the "why?" text with its fixes.

use puw_core::{Backend, CpuReason, DeviceReport};
use serde::{Deserialize, Serialize};

/// What the device chip shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceState {
    /// Every loaded model runs on the graphics card (Core ML on Macs, DirectML on Windows).
    Gpu,
    /// At least one loaded model runs on the processor.
    Cpu,
    /// No model is loaded yet.
    NotStarted,
}

/// A button the "why?" dialog offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceFix {
    /// Load the models again, trying the graphics card first.
    TryAgain,
    /// Use the processor from now on.
    UseProcessor,
    /// Use the graphics card from now on.
    UseGraphicsCard,
    /// Download the graphics card helper. Never offered yet: Plan 3 adds the helper.
    DownloadHelper { bytes: u64 },
}

/// The device chip's state, the sentence behind its "why?", the raw text for "Show details"
/// and the fixes on offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStatus {
    pub state: DeviceState,
    pub reason: Option<String>,
    pub details: Option<String>,
    pub fixes: Vec<DeviceFix>,
}

/// The system the app runs on, as far as the device wording cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Windows,
    Other,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Other
        }
    }
}

/// The models in the order their reason is preferred when several run on the processor for the
/// same kind of reason: the remover does most of the work.
fn models(report: &DeviceReport) -> [(&'static str, Option<&Backend>); 3] {
    [
        ("watermark remover", report.lama.as_ref()),
        ("watermark finder", report.segmenter.as_ref()),
        ("face finder", report.face.as_ref()),
    ]
}

/// How strongly a reason should be the one shown: errors the person can act on come first.
fn rank(reason: &CpuReason) -> u8 {
    match reason {
        CpuReason::RunFailed(_) => 0,
        CpuReason::LoadFailed(_) => 1,
        CpuReason::ProviderUnavailable(_) => 2,
        CpuReason::PartialGpu { .. } => 3,
        CpuReason::NotRequested => 4,
    }
}

/// What the engine reports on an Intel Mac (`puw_core` engine, macOS platform module).
const NO_NEURAL_ENGINE: &str = "This Mac has no Neural Engine";

/// What DirectML says on a PC without a graphics card it can use (seen on a virtual machine
/// with only the basic display adapter). Trying again cannot help there.
const NO_DEVICE: &str = "no devices detected";

/// Whether the error text says the system has no graphics device the provider can use.
fn no_device(text: &str) -> bool {
    text.to_lowercase().contains(NO_DEVICE)
}

/// The hardware the reasons talk about: Macs use the Neural Engine through Core ML.
fn hardware(platform: Platform) -> &'static str {
    if platform == Platform::MacOs {
        "Neural Engine"
    } else {
        "graphics card"
    }
}

/// The raw text of a reason, for "Show details"; `None` when there is nothing more to say.
fn raw(reason: &CpuReason, platform: Platform) -> Option<String> {
    match reason {
        CpuReason::NotRequested => None,
        CpuReason::ProviderUnavailable(t) | CpuReason::LoadFailed(t) | CpuReason::RunFailed(t) => {
            Some(t.clone())
        }
        CpuReason::PartialGpu {
            gpu_nodes,
            total_nodes,
        } => Some(format!(
            "the {} can run {gpu_nodes} of {total_nodes} steps",
            hardware(platform)
        )),
    }
}

/// The sentence and fixes for a model on the processor. Macs say "Neural Engine" where other
/// systems say "graphics card".
fn explain(reason: &CpuReason, platform: Platform) -> (String, Vec<DeviceFix>) {
    use DeviceFix::*;
    let mac = platform == Platform::MacOs;
    let hw = hardware(platform);
    match reason {
        CpuReason::NotRequested if mac => {
            ("You chose the processor.".into(), vec![UseGraphicsCard])
        }
        CpuReason::NotRequested => (
            "You chose the processor in Settings.".into(),
            vec![UseGraphicsCard],
        ),
        CpuReason::ProviderUnavailable(t) if mac && t == NO_NEURAL_ENGINE => (
            "This Mac has no Neural Engine, so the processor is used.".into(),
            vec![],
        ),
        CpuReason::ProviderUnavailable(_) if mac => (
            "The Neural Engine could not be used.".into(),
            vec![TryAgain],
        ),
        CpuReason::ProviderUnavailable(t) | CpuReason::LoadFailed(t) if no_device(t) => (
            "This computer has no graphics card the app can use, so the processor is used.".into(),
            vec![],
        ),
        CpuReason::ProviderUnavailable(_) => (
            "This computer's graphics card could not be used.".into(),
            vec![TryAgain],
        ),
        CpuReason::LoadFailed(_) => (
            format!("The {hw} could not load the watermark remover."),
            vec![TryAgain, UseProcessor],
        ),
        CpuReason::PartialGpu { .. } => (
            format!(
                "Your {hw} can run only part of the watermark remover, so the processor runs all of it."
            ),
            vec![UseProcessor],
        ),
        CpuReason::RunFailed(_) => (
            format!("The {hw} stopped working, so the app switched to the processor."),
            vec![TryAgain],
        ),
    }
}

/// Lines of "Show details" text, `None` when there are none.
fn joined(lines: Vec<String>) -> Option<String> {
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// The device chip's status for `report` (`None` before the engine exists). `Gpu` only when
/// every loaded model runs on the graphics card. With `prefer_gpu` off the reason is always the
/// person's own choice.
pub fn device_status(
    report: Option<&DeviceReport>,
    prefer_gpu: bool,
    platform: Platform,
) -> DeviceStatus {
    let loaded: Vec<(&str, &Backend)> = report
        .map(|r| {
            models(r)
                .into_iter()
                .filter_map(|(label, b)| b.map(|b| (label, b)))
                .collect()
        })
        .unwrap_or_default();
    if loaded.is_empty() {
        return DeviceStatus {
            state: DeviceState::NotStarted,
            reason: None,
            details: None,
            fixes: vec![],
        };
    }

    let on_cpu: Vec<(&str, &CpuReason)> = loaded
        .iter()
        .filter_map(|&(label, b)| match b {
            Backend::Cpu { reason } => Some((label, reason)),
            Backend::Gpu { .. } => None,
        })
        .collect();

    if on_cpu.is_empty() {
        let shares: Vec<String> = loaded
            .iter()
            .filter_map(|&(label, b)| match b {
                Backend::Gpu {
                    name,
                    share: Some((gpu, total)),
                } => Some(format!("{label}: {name} runs {gpu} of {total} steps")),
                _ => None,
            })
            .collect();
        let reason = (!shares.is_empty()).then(|| {
            let hardware = if platform == Platform::MacOs {
                "the Neural Engine"
            } else {
                "the graphics hardware"
            };
            format!("Part of the work runs on {hardware} and part on the processor.")
        });
        return DeviceStatus {
            state: DeviceState::Gpu,
            reason,
            details: joined(shares),
            fixes: vec![],
        };
    }

    let shown = if prefer_gpu {
        on_cpu
            .iter()
            .map(|&(_, r)| r)
            .min_by_key(|r| rank(r))
            .cloned()
            .unwrap_or(CpuReason::NotRequested)
    } else {
        CpuReason::NotRequested
    };
    let (sentence, fixes) = explain(&shown, platform);
    let details = on_cpu
        .iter()
        .filter_map(|&(label, r)| raw(r, platform).map(|t| format!("{label}: {t}")))
        .collect();
    DeviceStatus {
        state: DeviceState::Cpu,
        reason: Some(sentence),
        details: joined(details),
        fixes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DeviceFix::*;

    fn gpu(name: &str) -> Backend {
        Backend::Gpu {
            name: name.into(),
            share: None,
        }
    }

    fn cpu(reason: CpuReason) -> Backend {
        Backend::Cpu { reason }
    }

    fn report(
        segmenter: Option<Backend>,
        lama: Option<Backend>,
        face: Option<Backend>,
    ) -> DeviceReport {
        DeviceReport {
            segmenter,
            lama,
            face,
        }
    }

    /// Every model on `b`.
    fn all(b: Backend) -> DeviceReport {
        report(Some(b.clone()), Some(b.clone()), Some(b))
    }

    fn assert_plain(text: &str) {
        let lower = text.to_lowercase();
        for w in lower.split(|c: char| !c.is_alphanumeric()) {
            assert!(!["mask", "mark", "unmark"].contains(&w), "{text}");
        }
        for p in [
            "inpaint",
            "segment",
            "threshold",
            "dilat",
            "onnx",
            "tensor",
            "core ml",
            "directml",
        ] {
            assert!(!lower.contains(p), "{text}");
        }
        assert!(text.is_ascii(), "{text}");
    }

    #[test]
    fn device_status_cases() {
        let w = Platform::Windows;
        let s = |r: &DeviceReport| device_status(Some(r), true, w);

        // Nothing loaded yet.
        for st in [
            device_status(None, true, w),
            device_status(Some(&report(None, None, None)), true, w),
            device_status(None, false, Platform::MacOs),
        ] {
            assert_eq!(st.state, DeviceState::NotStarted);
            assert_eq!((st.reason, st.details, st.fixes), (None, None, vec![]));
        }

        // All on the graphics card, whole models: nothing to explain.
        let st = s(&all(gpu("DirectML")));
        assert_eq!(st.state, DeviceState::Gpu);
        assert_eq!((st.reason, st.details, st.fixes), (None, None, vec![]));
        let st = s(&report(None, Some(gpu("DirectML")), None));
        assert_eq!(st.state, DeviceState::Gpu);

        // Per reason, on Windows.
        let cases = [
            (
                CpuReason::NotRequested,
                "You chose the processor in Settings.",
                None,
                vec![UseGraphicsCard],
            ),
            (
                CpuReason::ProviderUnavailable("no adapter".into()),
                "This computer's graphics card could not be used.",
                Some("no adapter"),
                vec![TryAgain],
            ),
            (
                CpuReason::LoadFailed("bad graph".into()),
                "The graphics card could not load the watermark remover.",
                Some("bad graph"),
                vec![TryAgain, UseProcessor],
            ),
            (
                CpuReason::ProviderUnavailable(
                    "DirectML: No devices detected that match the filter criteria.".into(),
                ),
                "This computer has no graphics card the app can use, so the processor is used.",
                Some("No devices detected"),
                vec![],
            ),
            (
                CpuReason::LoadFailed("No devices detected that match the filter criteria".into()),
                "This computer has no graphics card the app can use, so the processor is used.",
                Some("No devices detected"),
                vec![],
            ),
            (
                CpuReason::PartialGpu {
                    gpu_nodes: 10,
                    total_nodes: 12,
                },
                "Your graphics card can run only part of the watermark remover, so the processor runs all of it.",
                Some("10 of 12"),
                vec![UseProcessor],
            ),
            (
                CpuReason::RunFailed("device removed".into()),
                "The graphics card stopped working, so the app switched to the processor.",
                Some("device removed"),
                vec![TryAgain],
            ),
        ];
        for (reason, sentence, detail, fixes) in cases {
            let prefer_gpu = reason != CpuReason::NotRequested;
            let st = device_status(Some(&all(cpu(reason.clone()))), prefer_gpu, w);
            assert_eq!(st.state, DeviceState::Cpu, "{reason:?}");
            assert_eq!(st.reason.as_deref(), Some(sentence), "{reason:?}");
            assert_eq!(st.fixes, fixes, "{reason:?}");
            match detail {
                Some(d) => {
                    let details = st.details.unwrap();
                    assert!(details.contains(d), "{reason:?}: {details}");
                    assert!(details.contains("watermark remover: "), "{details}");
                }
                None => assert_eq!(st.details, None, "{reason:?}"),
            }
            assert_plain(sentence);
        }
    }

    #[test]
    fn device_status_mixed_is_cpu_with_the_strongest_reason() {
        let r = report(
            Some(cpu(CpuReason::PartialGpu {
                gpu_nodes: 1,
                total_nodes: 2,
            })),
            Some(gpu("DirectML")),
            Some(cpu(CpuReason::RunFailed("lost".into()))),
        );
        let st = device_status(Some(&r), true, Platform::Windows);
        assert_eq!(st.state, DeviceState::Cpu);
        assert_eq!(
            st.reason.as_deref(),
            Some("The graphics card stopped working, so the app switched to the processor.")
        );
        assert_eq!(st.fixes, vec![TryAgain]);
        let details = st.details.unwrap();
        assert!(
            details.contains("watermark finder: the graphics card can run 1 of 2 steps"),
            "{details}"
        );
        assert!(details.contains("face finder: lost"), "{details}");
        assert!(!details.contains("watermark remover"), "{details}");
    }

    #[test]
    fn device_status_says_the_processor_was_chosen_when_it_was() {
        let r = all(cpu(CpuReason::ProviderUnavailable("none".into())));
        let st = device_status(Some(&r), false, Platform::Windows);
        assert_eq!(
            st.reason.as_deref(),
            Some("You chose the processor in Settings.")
        );
        assert_eq!(st.fixes, vec![UseGraphicsCard]);
        assert_eq!(
            st.details.as_deref(),
            Some("watermark remover: none\nwatermark finder: none\nface finder: none")
        );
    }

    #[test]
    fn device_status_on_macs() {
        let mac = Platform::MacOs;
        // Intel Macs: no Neural Engine, nothing to fix.
        let r = all(cpu(CpuReason::ProviderUnavailable(
            "This Mac has no Neural Engine".into(),
        )));
        let st = device_status(Some(&r), true, mac);
        assert_eq!(st.state, DeviceState::Cpu);
        assert_eq!(
            st.reason.as_deref(),
            Some("This Mac has no Neural Engine, so the processor is used.")
        );
        assert_eq!(st.fixes, vec![]);
        assert!(
            st.details
                .unwrap()
                .contains("This Mac has no Neural Engine")
        );

        // Apple silicon: Core ML for every model, two of them in part.
        let r = report(
            Some(Backend::Gpu {
                name: "Core ML".into(),
                share: Some((3223, 3231)),
            }),
            Some(Backend::Gpu {
                name: "Core ML".into(),
                share: Some((1680, 2877)),
            }),
            Some(gpu("Core ML")),
        );
        let st = device_status(Some(&r), true, mac);
        assert_eq!(st.state, DeviceState::Gpu);
        let reason = st.reason.unwrap();
        assert_eq!(
            reason,
            "Part of the work runs on the Neural Engine and part on the processor."
        );
        assert_plain(&reason);
        assert_eq!(st.fixes, vec![]);
        assert_eq!(
            st.details.as_deref(),
            Some(
                "watermark remover: Core ML runs 1680 of 2877 steps\nwatermark finder: Core ML runs 3223 of 3231 steps"
            )
        );

        // A share elsewhere says "graphics hardware".
        let st = device_status(Some(&r), true, Platform::Windows);
        assert_eq!(
            st.reason.as_deref(),
            Some("Part of the work runs on the graphics hardware and part on the processor.")
        );
    }

    #[test]
    fn device_status_mac_reasons_say_neural_engine() {
        let mac = Platform::MacOs;
        let cases = [
            (
                CpuReason::NotRequested,
                "You chose the processor.",
                vec![UseGraphicsCard],
            ),
            (
                CpuReason::ProviderUnavailable(
                    "Core ML cannot run any of the 9 steps of the lama model".into(),
                ),
                "The Neural Engine could not be used.",
                vec![TryAgain],
            ),
            (
                // Only the exact Intel text means there is no Neural Engine.
                CpuReason::ProviderUnavailable("This Mac has no Neural Engine here".into()),
                "The Neural Engine could not be used.",
                vec![TryAgain],
            ),
            (
                CpuReason::LoadFailed("x".into()),
                "The Neural Engine could not load the watermark remover.",
                vec![TryAgain, UseProcessor],
            ),
            (
                CpuReason::PartialGpu {
                    gpu_nodes: 1,
                    total_nodes: 2,
                },
                "Your Neural Engine can run only part of the watermark remover, so the processor runs all of it.",
                vec![UseProcessor],
            ),
            (
                CpuReason::RunFailed("x".into()),
                "The Neural Engine stopped working, so the app switched to the processor.",
                vec![TryAgain],
            ),
        ];
        for (reason, sentence, fixes) in cases {
            let prefer_gpu = reason != CpuReason::NotRequested;
            let st = device_status(Some(&all(cpu(reason.clone()))), prefer_gpu, mac);
            assert_eq!(st.state, DeviceState::Cpu, "{reason:?}");
            assert_eq!(st.reason.as_deref(), Some(sentence), "{reason:?}");
            assert_eq!(st.fixes, fixes, "{reason:?}");
            assert!(!sentence.contains("graphics"), "{sentence}");
            assert_plain(sentence);
        }
        let partial = all(cpu(CpuReason::PartialGpu {
            gpu_nodes: 1,
            total_nodes: 2,
        }));
        let details = device_status(Some(&partial), true, mac).details.unwrap();
        assert!(
            details.contains("the Neural Engine can run 1 of 2 steps"),
            "{details}"
        );
    }

    #[test]
    fn device_status_serializes_for_the_interface() {
        let st = device_status(
            Some(&all(cpu(CpuReason::LoadFailed("x".into())))),
            true,
            Platform::Windows,
        );
        let json = serde_json::to_value(&st).unwrap();
        assert_eq!(json["state"], "cpu");
        assert_eq!(
            json["fixes"],
            serde_json::json!(["tryAgain", "useProcessor"])
        );
        assert_eq!(
            serde_json::to_value(DeviceState::NotStarted).unwrap(),
            "notStarted"
        );
        let fix: DeviceFix = serde_json::from_str("\"useGraphicsCard\"").unwrap();
        assert_eq!(fix, UseGraphicsCard);
        let helper: DeviceFix = serde_json::from_str(r#"{"downloadHelper":{"bytes":5}}"#).unwrap();
        assert_eq!(helper, DownloadHelper { bytes: 5 });
    }
}
