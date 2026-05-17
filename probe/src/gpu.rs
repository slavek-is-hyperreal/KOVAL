use std::ffi::CStr;
use std::fs;
use std::path::Path;
use schema::{GpuProfile, VulkanDeviceProfile};
use ash::vk;

pub fn collect() -> GpuProfile {
    let mut devices = Vec::new();

    unsafe {
        if let Ok(entry) = ash::Entry::load() {
            let app_name = CStr::from_bytes_with_nul(b"KovalProbe\0").unwrap();
            let app_info = vk::ApplicationInfo::default()
                .application_name(app_name)
                .application_version(1)
                .engine_name(app_name)
                .engine_version(1)
                .api_version(vk::API_VERSION_1_0);

            let create_info = vk::InstanceCreateInfo::default()
                .application_info(&app_info);

            if let Ok(instance) = entry.create_instance(&create_info, None) {
                if let Ok(phys_devs) = instance.enumerate_physical_devices() {
                    for (i, &phys_dev) in phys_devs.iter().enumerate() {
                        let props = instance.get_physical_device_properties(phys_dev);
                        if props.device_type == vk::PhysicalDeviceType::CPU {
                            continue;
                        }

                        let name = CStr::from_ptr(props.device_name.as_ptr())
                            .to_string_lossy()
                            .into_owned();

                        let mem_props = instance.get_physical_device_memory_properties(phys_dev);
                        let mut vram_bytes = 0;
                        for heap_idx in 0..mem_props.memory_heap_count as usize {
                            let heap = mem_props.memory_heaps[heap_idx];
                            if heap.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL) {
                                vram_bytes += heap.size;
                            }
                        }

                        let pcie_info = read_pcie_info_for_card(&format!("card{}", i));

                        devices.push(VulkanDeviceProfile {
                            name,
                            vram_bytes,
                            pcie_info,
                        });
                    }
                }
                instance.destroy_instance(None);
            }
        }
    }

    if devices.is_empty() {
        if let Ok(entries) = fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("card") && !name.contains('-') {
                    let pcie_info = read_pcie_info_for_card(&name);
                    devices.push(VulkanDeviceProfile {
                        name: format!("DRM Graphics Device ({})", name),
                        vram_bytes: 0,
                        pcie_info,
                    });
                    break;
                }
            }
        }
    }

    GpuProfile { devices }
}

fn read_pcie_info_for_card(card_name: &str) -> Option<String> {
    let device_dir = format!("/sys/class/drm/{}/device", card_name);
    let path_width = format!("{}/current_link_width", device_dir);
    let path_speed = format!("{}/current_link_speed", device_dir);

    if Path::new(&path_width).exists() && Path::new(&path_speed).exists() {
        if let (Ok(width), Ok(speed)) = (fs::read_to_string(path_width), fs::read_to_string(path_speed)) {
            return Some(format!("PCIe Link: x{} @ {}", width.trim(), speed.trim()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_gpu_does_not_panic() {
        let profile = collect();
        println!("Collected {} GPU devices", profile.devices.len());
    }

    #[test]
    #[cfg(feature = "live_gpu_tests")]
    fn test_live_vulkan_enumeration() {
        unsafe {
            let entry = ash::Entry::load().expect("Vulkan library not loaded");
            let app_name = CStr::from_bytes_with_nul(b"KovalProbe\0").unwrap();
            let app_info = vk::ApplicationInfo::default()
                .application_name(app_name)
                .api_version(vk::API_VERSION_1_0);
            let create_info = vk::InstanceCreateInfo::default()
                .application_info(&app_info);
            let instance = entry.create_instance(&create_info, None).expect("Instance creation failed");
            let phys_devs = instance.enumerate_physical_devices().expect("Enumeration failed");
            for dev in phys_devs {
                let props = instance.get_physical_device_properties(dev);
                let name = CStr::from_ptr(props.device_name.as_ptr()).to_string_lossy();
                println!("Device: {}", name);
            }
            instance.destroy_instance(None);
        }
    }
}
