//! Tab content variants — one per [`DeviceClass`](rb_device::DeviceClass).
//!
//! Each variant holds the device-class-specific state for a [`TabState`](crate::tab_state::TabState).
//! New variants are added as sibling modules to [`crate::logic_analyzer`].

use rb_device::DeviceClass;

pub use crate::logic_analyzer::LogicAnalyzerContent;

/// Device-class-specific content inside a tab.
///
/// Determined once when the tab's [`TabSource`](crate::tab_state::TabSource) is set.
/// For a multi-class device the user picks one class; open another tab for the other.
pub enum TabContent {
    /// Logic Analyzer: digital/analog acquisition, waveform display.
    LogicAnalyzer(LogicAnalyzerContent),
    // Future:
    // WaveformGenerator(WaveformGenContent),
    // Oscilloscope(OscilloscopeContent),
    // SdrReceiver(SdrReceiverContent),
}

impl TabContent {
    /// Creates the default content for a given device class.
    #[must_use]
    pub fn from_device_class(class: DeviceClass) -> Self {
        match class {
            DeviceClass::LogicAnalyzer => {
                TabContent::LogicAnalyzer(LogicAnalyzerContent::default())
            }
            // Future variants will be added here.
            _ => panic!("TabContent not yet implemented for {class:?}"),
        }
    }

    /// The [`DeviceClass`] this content represents.
    #[must_use]
    pub fn device_class(&self) -> DeviceClass {
        match self {
            TabContent::LogicAnalyzer(_) => DeviceClass::LogicAnalyzer,
        }
    }

    /// Whether this content is currently running (acquiring, generating, …).
    #[must_use]
    pub fn is_running(&self) -> bool {
        match self {
            TabContent::LogicAnalyzer(la) => la.is_running(),
        }
    }
}

impl Default for TabContent {
    fn default() -> Self {
        TabContent::LogicAnalyzer(LogicAnalyzerContent::default())
    }
}
