//! NVIDIA's NVML, loaded at run time. This is the vendor path for a machine
//! without LibreHardwareMonitor: the driver ships `nvml.dll`, and every call
//! here is a read.
//!
//! The provider is registered before LHM, so a card NVML can measure answers
//! from the driver and LHM fills only the keys NVML cannot. An unavailable
//! NVML reports nothing, so LHM (or a missing reading) answers unchanged.

#![deny(unsafe_op_in_unsafe_fn)]

use std::sync::Arc;

use yunari_providers::devices::ids_for;
use yunari_providers::provider::{Descriptor, Health, Provider};
use yunari_providers::sample::Reading;
use yunari_providers::vocabulary;

/// Every key this provider can answer.
pub const KEYS: &[&str] = &[
    "gpu.load",
    "gpu.temp",
    "gpu.temp.core",
    "gpu.power",
    "gpu.clock",
    "gpu.clock.memory",
    "gpu.name",
    "vram.used",
    "vram.used.percent",
    "vram.total",
];

/// One card as NVML describes it. A figure the driver refuses is absent, not
/// zero.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Gpu {
    pub name: String,
    pub load: Option<f64>,
    pub temperature: Option<f64>,
    pub power: Option<f64>,
    pub clock: Option<f64>,
    pub memory_clock: Option<f64>,
    pub memory_used: Option<f64>,
    pub memory_total: Option<f64>,
}

/// Reads every card this driver can see.
pub fn snapshot() -> Result<Vec<Gpu>, String> {
    platform::snapshot()
}

type Source = Arc<dyn Fn() -> Result<Vec<Gpu>, String> + Send + Sync>;

pub struct NvmlProvider {
    source: Source,
    devices: yunari_providers::devices::Loader,
}

impl NvmlProvider {
    pub fn new() -> Self {
        Self {
            source: Arc::new(snapshot),
            devices: Arc::new(yunari_providers::devices::Assignment::default),
        }
    }

    /// A provider over another source, for a test that must not need a card.
    pub fn with_source(
        source: impl Fn() -> Result<Vec<Gpu>, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            source: Arc::new(source),
            devices: Arc::new(yunari_providers::devices::Assignment::default),
        }
    }

    pub fn with_devices(mut self, devices: yunari_providers::devices::Loader) -> Self {
        self.devices = devices;
        self
    }
}

impl Default for NvmlProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for NvmlProvider {
    fn id(&self) -> &str {
        "nvml"
    }

    fn label(&self) -> &str {
        "NVIDIA (NVML)"
    }

    fn describe(&self) -> Vec<Descriptor> {
        if (self.source)().is_err() {
            return Vec::new();
        }
        KEYS.iter()
            .map(|key| {
                Descriptor::new(*key, vocabulary::describe(key).and_then(|entry| entry.unit))
            })
            .collect()
    }

    fn devices(&self) -> Vec<yunari_providers::provider::Device> {
        let Ok(gpus) = (self.source)() else {
            return Vec::new();
        };
        ids_for(gpus.into_iter().map(|gpu| gpu.name))
            .into_iter()
            .map(|(name, id)| yunari_providers::provider::Device {
                group: "gpu".to_owned(),
                id,
                name,
            })
            .collect()
    }

    fn sample(&self, keys: &[String], _now_ms: u64) -> Result<Vec<Reading>, String> {
        let gpus = match (self.source)() {
            Ok(gpus) => gpus,
            // An absent driver is a standing the health row already states; a
            // poll failure every cycle would say it as an error instead.
            Err(_) => return Ok(Vec::new()),
        };
        let assignment = (self.devices)();
        let chosen = match assignment.gpu.as_deref() {
            Some(assigned) => {
                let listed = ids_for(gpus.iter().map(|gpu| gpu.name.clone()));
                let ids: Vec<String> = listed.iter().map(|(_, id)| id.clone()).collect();
                let names: Vec<String> = listed.iter().map(|(name, _)| name.clone()).collect();
                match yunari_providers::devices::index_of(&ids, &names, &assignment, assigned) {
                    Some(index) => index,
                    None => {
                        return Ok(keys
                            .iter()
                            .filter(|key| KEYS.contains(&key.as_str()))
                            .map(|key| {
                                Reading::missing(
                                    key,
                                    "The assigned GPU is not present on this machine.",
                                )
                            })
                            .collect());
                    }
                }
            }
            None => 0,
        };
        let Some(gpu) = gpus.get(chosen) else {
            return Ok(Vec::new());
        };
        let mut readings = Vec::new();
        for key in keys {
            let value = match key.as_str() {
                "gpu.load" => gpu.load,
                "gpu.temp" | "gpu.temp.core" => gpu.temperature,
                "gpu.power" => gpu.power,
                "gpu.clock" => gpu.clock,
                "gpu.clock.memory" => gpu.memory_clock,
                "vram.used" => gpu.memory_used,
                "vram.total" => gpu.memory_total,
                "vram.used.percent" => match (gpu.memory_used, gpu.memory_total) {
                    (Some(used), Some(total)) if total > 0.0 => Some(used / total * 100.0),
                    _ => None,
                },
                "gpu.name" => {
                    readings.push(Reading::text(key.clone(), gpu.name.clone()));
                    continue;
                }
                _ => continue,
            };
            let Some(value) = value else {
                continue;
            };
            let unit = vocabulary::describe(key)
                .and_then(|entry| entry.unit)
                .unwrap_or_default();
            readings.push(Reading::number(
                key.clone(),
                (value * 100.0).round() / 100.0,
                unit,
            ));
        }
        Ok(readings)
    }

    fn health(&self) -> Health {
        match (self.source)() {
            Ok(_) => Health::Ok,
            Err(reason) => Health::Unavailable(reason),
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::{CString, c_char, c_void};

    use windows_sys::Win32::Foundation::FreeLibrary;
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LoadLibraryExW,
    };

    use super::Gpu;

    const SUCCESS: i32 = 0;
    /// `NVML_TEMPERATURE_GPU`
    const TEMPERATURE_GPU: u32 = 0;
    /// `NVML_CLOCK_GRAPHICS` and `NVML_CLOCK_MEM`
    const CLOCK_GRAPHICS: u32 = 0;
    const CLOCK_MEMORY: u32 = 2;

    #[repr(C)]
    struct Utilization {
        gpu: u32,
        memory: u32,
    }

    #[repr(C)]
    struct Memory {
        total: u64,
        free: u64,
        used: u64,
    }

    type Init = unsafe extern "C" fn() -> i32;
    type Shutdown = unsafe extern "C" fn() -> i32;
    type Count = unsafe extern "C" fn(*mut u32) -> i32;
    type Handle = unsafe extern "C" fn(u32, *mut *mut c_void) -> i32;
    type Name = unsafe extern "C" fn(*mut c_void, *mut c_char, u32) -> i32;
    type Util = unsafe extern "C" fn(*mut c_void, *mut Utilization) -> i32;
    type Mem = unsafe extern "C" fn(*mut c_void, *mut Memory) -> i32;
    type Temp = unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> i32;
    type Power = unsafe extern "C" fn(*mut c_void, *mut u32) -> i32;
    type Clock = unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> i32;

    struct Library {
        module: *mut c_void,
    }

    impl Drop for Library {
        fn drop(&mut self) {
            // SAFETY: the handle came from `LoadLibraryExW` and is freed once.
            unsafe { FreeLibrary(self.module) };
        }
    }

    impl Library {
        fn open() -> Option<Self> {
            let wide: Vec<u16> = "nvml.dll"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            // SAFETY: the name is a NUL-terminated wide string that outlives
            // the call.
            let module = unsafe {
                LoadLibraryExW(
                    wide.as_ptr(),
                    std::ptr::null_mut(),
                    LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
                )
            };
            (!module.is_null()).then_some(Self { module })
        }

        /// # Safety
        /// The caller names a symbol with the signature the driver exports.
        unsafe fn symbol<T>(&self, name: &str) -> Option<T> {
            let name = CString::new(name).ok()?;
            // SAFETY: the module is live for `self`'s lifetime and the name is
            // a NUL-terminated string.
            let address = unsafe { GetProcAddress(self.module, name.as_ptr().cast()) }?;
            Some(unsafe { std::mem::transmute_copy::<_, T>(&address) })
        }
    }

    pub fn snapshot() -> Result<Vec<Gpu>, String> {
        let library = Library::open().ok_or("The NVIDIA driver's nvml.dll is not installed.")?;
        // SAFETY: every symbol is looked up by the name NVML documents, and the
        // calls below pass live buffers of the documented size.
        unsafe { read(&library) }
    }

    unsafe fn read(library: &Library) -> Result<Vec<Gpu>, String> {
        let init: Init =
            unsafe { library.symbol("nvmlInit_v2") }.ok_or("nvml.dll has no nvmlInit_v2.")?;
        let shutdown: Shutdown =
            unsafe { library.symbol("nvmlShutdown") }.ok_or("nvml.dll has no nvmlShutdown.")?;
        let count: Count = unsafe { library.symbol("nvmlDeviceGetCount_v2") }
            .ok_or("nvml.dll has no nvmlDeviceGetCount_v2.")?;
        let handle: Handle = unsafe { library.symbol("nvmlDeviceGetHandleByIndex_v2") }
            .ok_or("nvml.dll has no nvmlDeviceGetHandleByIndex_v2.")?;
        let name: Name = unsafe { library.symbol("nvmlDeviceGetName") }
            .ok_or("nvml.dll has no nvmlDeviceGetName.")?;
        let utilization: Util = unsafe { library.symbol("nvmlDeviceGetUtilizationRates") }
            .ok_or("nvml.dll has no nvmlDeviceGetUtilizationRates.")?;
        let memory: Mem = unsafe { library.symbol("nvmlDeviceGetMemoryInfo") }
            .ok_or("nvml.dll has no nvmlDeviceGetMemoryInfo.")?;
        let temperature: Temp = unsafe { library.symbol("nvmlDeviceGetTemperature") }
            .ok_or("nvml.dll has no nvmlDeviceGetTemperature.")?;
        let power: Power = unsafe { library.symbol("nvmlDeviceGetPowerUsage") }
            .ok_or("nvml.dll has no nvmlDeviceGetPowerUsage.")?;
        let clock: Clock = unsafe { library.symbol("nvmlDeviceGetClockInfo") }
            .ok_or("nvml.dll has no nvmlDeviceGetClockInfo.")?;

        let status = unsafe { init() };
        if status != SUCCESS {
            return Err(format!("NVML could not start (error {status})."));
        }
        let mut total = 0u32;
        let status = unsafe { count(&mut total) };
        if status != SUCCESS {
            unsafe { shutdown() };
            return Err(format!("NVML could not count devices (error {status})."));
        }
        let mut gpus = Vec::new();
        for index in 0..total {
            let mut device: *mut c_void = std::ptr::null_mut();
            if unsafe { handle(index, &mut device) } != SUCCESS {
                continue;
            }
            let mut buffer = [0 as c_char; 96];
            let name =
                if unsafe { name(device, buffer.as_mut_ptr(), buffer.len() as u32) } == SUCCESS {
                    // SAFETY: NVML wrote a NUL-terminated name into the buffer.
                    let text = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) };
                    text.to_string_lossy().into_owned()
                } else {
                    format!("NVIDIA GPU {index}")
                };
            let mut gpu = Gpu {
                name,
                ..Gpu::default()
            };
            let mut usage = Utilization { gpu: 0, memory: 0 };
            if unsafe { utilization(device, &mut usage) } == SUCCESS {
                gpu.load = Some(f64::from(usage.gpu));
            }
            let mut info = Memory {
                total: 0,
                free: 0,
                used: 0,
            };
            if unsafe { memory(device, &mut info) } == SUCCESS && info.total > 0 {
                gpu.memory_total = Some(info.total as f64 / 1e9);
                gpu.memory_used = Some(info.used as f64 / 1e9);
            }
            let mut celsius = 0u32;
            if unsafe { temperature(device, TEMPERATURE_GPU, &mut celsius) } == SUCCESS {
                gpu.temperature = Some(f64::from(celsius));
            }
            let mut milliwatts = 0u32;
            if unsafe { power(device, &mut milliwatts) } == SUCCESS {
                gpu.power = Some(f64::from(milliwatts) / 1000.0);
            }
            let mut megahertz = 0u32;
            if unsafe { clock(device, CLOCK_GRAPHICS, &mut megahertz) } == SUCCESS {
                gpu.clock = Some(f64::from(megahertz));
            }
            if unsafe { clock(device, CLOCK_MEMORY, &mut megahertz) } == SUCCESS {
                gpu.memory_clock = Some(f64::from(megahertz));
            }
            gpus.push(gpu);
        }
        unsafe { shutdown() };
        Ok(gpus)
    }
}

#[cfg(not(windows))]
mod platform {
    use super::Gpu;

    pub fn snapshot() -> Result<Vec<Gpu>, String> {
        Err("NVML is available on Windows only.".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(gpus: Vec<Gpu>) -> NvmlProvider {
        NvmlProvider::with_source(move || Ok(gpus.clone()))
    }

    #[test]
    fn a_card_answers_the_vendor_keys_in_their_units() {
        let provider = provider(vec![Gpu {
            name: "NVIDIA GeForce RTX 4090".to_owned(),
            load: Some(41.0),
            temperature: Some(62.0),
            power: Some(210.5),
            clock: Some(2520.0),
            memory_clock: Some(10501.0),
            memory_used: Some(4.0),
            memory_total: Some(24.0),
        }]);
        let keys: Vec<String> = KEYS.iter().map(|key| (*key).to_owned()).collect();
        let readings = provider.sample(&keys, 0).expect("a sample");
        let value = |key: &str| {
            readings
                .iter()
                .find(|reading| reading.semantic_key == key)
                .and_then(|reading| reading.value)
        };
        assert_eq!(value("gpu.load"), Some(41.0));
        assert_eq!(value("gpu.temp"), Some(62.0));
        assert_eq!(value("gpu.power"), Some(210.5));
        assert_eq!(value("vram.used"), Some(4.0));
        assert_eq!(value("vram.used.percent"), Some(16.67));
        let name = readings
            .iter()
            .find(|reading| reading.semantic_key == "gpu.name")
            .and_then(|reading| reading.text.clone());
        assert_eq!(name.as_deref(), Some("NVIDIA GeForce RTX 4090"));
    }

    #[test]
    #[ignore = "needs an NVIDIA driver on this machine"]
    fn this_machine_answers_from_its_driver() {
        let gpus = snapshot().expect("the driver");
        assert!(!gpus.is_empty());
        assert!(
            gpus[0].memory_total.is_some_and(|total| total > 0.0),
            "{:?}",
            gpus[0]
        );
        assert!(gpus[0].temperature.is_some());
    }

    #[test]
    fn a_missing_vendor_library_is_one_reason_and_no_keys() {
        let provider = NvmlProvider::with_source(|| Err("no driver".to_owned()));
        assert!(provider.describe().is_empty());
        assert_eq!(
            provider.health(),
            Health::Unavailable("no driver".to_owned())
        );
        // The sample is an error so the registry records the reason, and the
        // next provider still answers.
        assert!(
            provider
                .sample(&["gpu.load".to_owned()], 0)
                .expect("an absent driver is not a poll failure")
                .is_empty()
        );
    }
}
