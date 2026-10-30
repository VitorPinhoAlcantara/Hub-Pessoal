use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct GpuSnapshot {
    pub name: String,
    pub usage: f32,
    pub vram_used: u64,
    pub vram_total: u64,
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn parse_luid(instance: &str) -> Option<u64> {
    let rest = &instance[instance.find("luid_0x")? + "luid_0x".len()..];
    let (high, rest) = rest.split_once("_0x")?;
    let low: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
    let high = u32::from_str_radix(high, 16).ok()?;
    let low = u32::from_str_radix(&low, 16).ok()?;
    Some(((high as u64) << 32) | low as u64)
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn sum_by_luid(samples: &[(String, f64)]) -> HashMap<u64, f64> {
    let mut totals = HashMap::new();
    for (instance, value) in samples {
        if let Some(luid) = parse_luid(instance) {
            *totals.entry(luid).or_insert(0.0) += value;
        }
    }
    totals
}

pub struct GpuReader(imp::Reader);

impl GpuReader {
    pub fn new() -> Option<Self> {
        imp::Reader::new().map(Self)
    }

    pub fn read(&mut self) -> Option<GpuSnapshot> {
        self.0.read()
    }
}

#[cfg(windows)]
mod imp {
    use std::slice;

    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1,
    };
    use windows::Win32::System::Performance::{
        PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
        PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCloseQuery,
        PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
    };
    use windows::core::w;

    use super::{GpuSnapshot, sum_by_luid};

    struct Adapter {
        luid: u64,
        name: String,
        vram_total: u64,
    }

    pub struct Reader {
        query: PDH_HQUERY,
        engine: PDH_HCOUNTER,
        memory: PDH_HCOUNTER,
        adapter: Adapter,
    }

    fn pick_adapter() -> Option<Adapter> {
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.ok()?;
        let mut best: Option<Adapter> = None;
        let mut index = 0;
        while let Ok(adapter) = unsafe { factory.EnumAdapters1(index) } {
            index += 1;
            let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
                continue;
            };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let vram_total = desc.DedicatedVideoMemory as u64;
            if best.as_ref().is_some_and(|b| b.vram_total >= vram_total) {
                continue;
            }
            let end = desc
                .Description
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(desc.Description.len());
            best = Some(Adapter {
                luid: ((desc.AdapterLuid.HighPart as u32 as u64) << 32)
                    | desc.AdapterLuid.LowPart as u64,
                name: String::from_utf16_lossy(&desc.Description[..end]),
                vram_total,
            });
        }
        best
    }

    fn read_array(counter: PDH_HCOUNTER) -> Vec<(String, f64)> {
        let mut size = 0u32;
        let mut count = 0u32;
        let status = unsafe {
            PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, None)
        };
        if status != PDH_MORE_DATA || size == 0 {
            return Vec::new();
        }
        let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
        let items = buffer.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
        let status = unsafe {
            PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_DOUBLE,
                &mut size,
                &mut count,
                Some(items),
            )
        };
        if status != 0 {
            return Vec::new();
        }
        unsafe { slice::from_raw_parts(items, count as usize) }
            .iter()
            .filter(|item| {
                item.FmtValue.CStatus == PDH_CSTATUS_VALID_DATA
                    || item.FmtValue.CStatus == PDH_CSTATUS_NEW_DATA
            })
            .filter_map(|item| {
                let name = unsafe { item.szName.to_string() }.ok()?;
                Some((name, unsafe { item.FmtValue.Anonymous.doubleValue }))
            })
            .collect()
    }

    impl Reader {
        pub fn new() -> Option<Self> {
            let adapter = pick_adapter()?;
            let mut query = PDH_HQUERY::default();
            if unsafe { PdhOpenQueryW(None, 0, &mut query) } != 0 {
                return None;
            }
            let mut engine = PDH_HCOUNTER::default();
            let mut memory = PDH_HCOUNTER::default();
            let added = unsafe {
                PdhAddEnglishCounterW(
                    query,
                    w!("\\GPU Engine(*engtype_3D)\\Utilization Percentage"),
                    0,
                    &mut engine,
                ) == 0
                    && PdhAddEnglishCounterW(
                        query,
                        w!("\\GPU Adapter Memory(*)\\Dedicated Usage"),
                        0,
                        &mut memory,
                    ) == 0
            };
            if !added {
                unsafe { PdhCloseQuery(query) };
                return None;
            }
            unsafe { PdhCollectQueryData(query) };
            Some(Self {
                query,
                engine,
                memory,
                adapter,
            })
        }

        pub fn read(&mut self) -> Option<GpuSnapshot> {
            if unsafe { PdhCollectQueryData(self.query) } != 0 {
                return None;
            }
            let usage = sum_by_luid(&read_array(self.engine))
                .get(&self.adapter.luid)
                .copied()
                .unwrap_or(0.0);
            let vram_used = sum_by_luid(&read_array(self.memory))
                .get(&self.adapter.luid)
                .copied()
                .unwrap_or(0.0);
            Some(GpuSnapshot {
                name: self.adapter.name.clone(),
                usage: usage.clamp(0.0, 100.0) as f32,
                vram_used: (vram_used.max(0.0) as u64).min(self.adapter.vram_total),
                vram_total: self.adapter.vram_total,
            })
        }
    }

    impl Drop for Reader {
        fn drop(&mut self) {
            unsafe { PdhCloseQuery(self.query) };
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::GpuSnapshot;

    pub struct Reader;

    impl Reader {
        pub fn new() -> Option<Self> {
            None
        }

        pub fn read(&mut self) -> Option<GpuSnapshot> {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_engine_and_adapter_instance_names() {
        let engine = "pid_1234_luid_0x00000000_0x0000F3A1_phys_0_eng_0_engtype_3D";
        let adapter = "luid_0x00000001_0x0000F3A1_phys_0";
        assert_eq!(parse_luid(engine), Some(0xF3A1));
        assert_eq!(parse_luid(adapter), Some((1 << 32) | 0xF3A1));
        assert_eq!(parse_luid("sem_luid"), None);
        assert_eq!(parse_luid("luid_0xZZ_0x1"), None);
    }

    #[test]
    fn sums_instances_per_adapter() {
        let samples = vec![
            (
                "pid_1_luid_0x00000000_0x0000000A_phys_0_eng_0_engtype_3D".to_string(),
                10.0,
            ),
            (
                "pid_2_luid_0x00000000_0x0000000A_phys_0_eng_0_engtype_3D".to_string(),
                5.5,
            ),
            (
                "pid_3_luid_0x00000000_0x0000000B_phys_0_eng_0_engtype_3D".to_string(),
                40.0,
            ),
            ("invalido".to_string(), 99.0),
        ];
        let totals = sum_by_luid(&samples);
        assert_eq!(totals.len(), 2);
        assert_eq!(totals[&0xA], 15.5);
        assert_eq!(totals[&0xB], 40.0);
    }
}
