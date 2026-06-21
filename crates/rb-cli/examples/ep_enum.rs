fn main() {
    let devices = nusb::list_devices().unwrap();
    for dev in devices {
        let vid = dev.vendor_id();
        let pid = dev.product_id();
        if vid != 0x1D50 || pid != 0x608C { continue; }
        println!("Device: {:04X}:{:04X}", vid, pid);
        let d = dev.open().unwrap();
        match d.active_configuration() {
            Ok(cfg) => {
                for intf in cfg.interfaces() {
                    for alt in intf.alt_settings() {
                        println!("  IF{} alt{}: class={} sub={} proto={}",
                            alt.interface_number(), alt.alternate_setting(),
                            alt.class(), alt.subclass(), alt.protocol());
                        for ep in alt.endpoints() {
                            let dir = if ep.direction() == nusb::transfer::Direction::In { "IN" } else { "OUT" };
                            println!("    EP 0x{:02X} {} max={}", ep.address(), dir, ep.max_packet_size());
                        }
                    }
                }
            }
            Err(e) => println!("  config error: {e}"),
        }
        let _ = d.claim_interface(0);
    }
}
