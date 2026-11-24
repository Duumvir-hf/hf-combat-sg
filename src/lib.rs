//! HighFleet Steam 1.163: a fleet-vs-fleet battle against a **strike group** loads `Combat_SG.seria`
//! instead of `Combat.seria`.
//!
//! HighFleet already tells the two kinds of engagement apart: the stock music picker
//! (`selectMusic_` `0x140253a50`) walks the enemy side of the engagement and, when a fleet there has
//! the behaviour word `+0x3a8 == 5` (a strike group), plays `Tanc_a_lelek` instead of the garrison
//! tracks. This mod reads that same list at the moment the battle level is chosen, so the level
//! switch and the music switch always agree.
//!
//! The level name is a literal at the single `Combat.seria` call site in `UpdateStrategicFleet`
//! (`lea rcx,[0x14035fa00]` at `0x140114688`). The hook replaces those seven bytes with a jump into a
//! trampoline that calls [`level_name`], then writes the returned pointer over RCX (or keeps the
//! stock pointer when the call returns 0) and falls through to the original
//! `call SetLevelSeriaToLoad_`. Everything else in the scene setup is untouched: the scenario is
//! picked from the `LVL_SCRIPT` key inside the file, not from the file name, so a copy of
//! `Combat.seria` behaves exactly like the stock battle.
//!
//! The mod refuses to switch when the level file is missing: the loader
//! (`App_ReloadSeria` `0x140240b90`) treats a failed `Data/Levels/<name>` load as fatal, logs
//! `ERR: reload_sg failed` and asks the game to exit.

highfleet_mod_api::export_logger!();

mod config;

use std::{
    ffi::{c_char, CStr},
    mem::{size_of, MaybeUninit},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use patchy::{PatchSession, ReturnType, Trampoline};
use windows_sys::Win32::System::{
    LibraryLoader::GetModuleHandleA,
    Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS},
};

const IMAGE_BASE: usize = 0x1_4000_0000;
const _: () = assert!(IMAGE_BASE == 0x140000000);

/// `lea rcx,[0x14035fa00]` inside `UpdateStrategicFleet`: puts the literal `"Combat.seria"` in RCX
/// as the level name of a fleet-vs-fleet engagement. The `call SetLevelSeriaToLoad_` is right after.
const LOAD_SITE: usize = 0x1_4011_4688;
const _: () = assert!(LOAD_SITE == 0x140114688);
const LOAD_SITE_BYTES: [u8; 7] = [0x48, 0x8d, 0x0d, 0x71, 0xb3, 0x24, 0x00];
/// First instruction after the replaced `lea`: the original `call 0x14025afd0`.
const RESUME: usize = 0x1_4011_468f;
const _: () = assert!(RESUME == 0x14011468f);

/// `std::vector<Link>` of the enemy side of the engagement - the list `selectMusic_` scans.
const ENEMY_SIDE_BEGIN: usize = 0x1_43a1_59c0;
const _: () = assert!(ENEMY_SIDE_BEGIN == 0x143a159c0);
const ENEMY_SIDE_END: usize = 0x1_43a1_59c8;
const _: () = assert!(ENEMY_SIDE_END == 0x143a159c8);
const LINK_STRIDE: usize = 0x18;
/// Offset of the referenced object inside a `Link` (the stock call site reads `[list[0]+8]` too).
const LINK_OBJECT: usize = 0x8;
/// Strategic fleet behaviour word; 5 is what `selectMusic_` treats as a strike group.
const FLEET_BEHAVIOUR: usize = 0x3a8;
const STRIKE_GROUP: i32 = 5;
/// Sanity bound: an engagement never holds more than a handful of fleets per side.
const MAX_LINKS: usize = 64;

static SUPPORTED_VERSION: AtomicBool = AtomicBool::new(false);
static VERBOSE: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// NUL-terminated level name handed to the game; allocated once at `init`.
static LEVEL_NAME: AtomicUsize = AtomicUsize::new(0);

fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

unsafe fn read<T: Copy>(address: usize) -> T {
    // SAFETY: every caller checks `readable` for the range first.
    unsafe { (address as *const T).read_unaligned() }
}

/// True when `size` bytes at `address` sit in a committed, readable, guard-free page.
fn readable(address: usize, size: usize) -> bool {
    let mut information = MaybeUninit::<MEMORY_BASIC_INFORMATION>::uninit();
    // SAFETY: VirtualQuery only inspects an address; the buffer is sized and aligned for the struct.
    let queried = unsafe {
        VirtualQuery(
            address as *const _,
            information.as_mut_ptr(),
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if queried != size_of::<MEMORY_BASIC_INFORMATION>() {
        return false;
    }
    // SAFETY: VirtualQuery returned a complete structure.
    let information = unsafe { information.assume_init() };
    if information.State != MEM_COMMIT
        || information.Protect == PAGE_NOACCESS
        || information.Protect & PAGE_GUARD != 0
    {
        return false;
    }
    let region_offset = address.saturating_sub(information.BaseAddress as usize);
    size <= information.RegionSize.saturating_sub(region_offset)
}

/// The enemy fleet that makes this engagement a strike-group fight, if any.
///
/// Mirrors the stock music picker exactly: first fleet on the enemy side of the engagement whose
/// behaviour word is [`STRIKE_GROUP`]. The list is the smart-reference vector the map update filled
/// just before it asked for the level, so it is the same data the battle scene will see.
unsafe fn strike_group() -> Option<usize> {
    // SAFETY: both are the game's own side-list globals.
    unsafe { strike_group_in(read(ENEMY_SIDE_BEGIN), read(ENEMY_SIDE_END)) }
}

unsafe fn strike_group_in(begin: usize, end: usize) -> Option<usize> {
    let usable = end > begin && begin >= 0x1_0000 && readable(begin, LINK_STRIDE);
    let count = if usable { (end - begin) / LINK_STRIDE } else { 0 };
    if count == 0 || count > MAX_LINKS {
        if verbose() {
            log::info!(
                "hf_combat_sg: enemy side list unusable (begin {begin:#x}, end {end:#x}); keeping Combat.seria"
            );
        }
        return None;
    }
    let mut states = [0i32; MAX_LINKS];
    let mut strike = None;
    for index in 0..count {
        let link = begin + index * LINK_STRIDE;
        if !readable(link, LINK_STRIDE) {
            break;
        }
        let fleet: usize = unsafe { read(link + LINK_OBJECT) };
        if fleet < 0x1_0000 || !readable(fleet + FLEET_BEHAVIOUR, 4) {
            states[index] = -1;
            continue;
        }
        let behaviour: i32 = unsafe { read(fleet + FLEET_BEHAVIOUR) };
        states[index] = behaviour;
        if behaviour == STRIKE_GROUP && strike.is_none() {
            strike = Some(fleet);
        }
    }
    if verbose() {
        log::info!(
            "hf_combat_sg: {count} enemy fleet(s), states {:?} -> {}",
            &states[..count],
            if strike.is_some() { "strike group" } else { "ordinary" }
        );
    }
    strike
}

/// Is the level file where the game will look for it (`Data/Levels/<name>`)?
///
/// The game loads it relative to its working directory, which is normally the game folder; the
/// executable's own folder is checked as well so a shortcut with another start-in path still counts.
fn level_exists(level: &str) -> bool {
    let relative = format!("Data/Levels/{level}");
    if std::path::Path::new(&relative).is_file() {
        return true;
    }
    match std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join(&relative))) {
        Some(path) => path.is_file(),
        None => false,
    }
}

/// The stock `"Combat.seria"` literal, decoded from the hooked instruction's own displacement at
/// install time. The replaced `lea` no longer runs, so the trampoline must supply this pointer on
/// every path that keeps the vanilla level: leaving RCX as the caller left it hands
/// `SetLevelSeriaToLoad_` a wild pointer and the game dies in its `strlen`.
static STOCK_NAME: AtomicUsize = AtomicUsize::new(0);

/// Hook body: the level name to use for this engagement, always a valid NUL-terminated string.
unsafe extern "C" fn level_name() -> usize {
    let stock = STOCK_NAME.load(Ordering::Relaxed);
    let level = LEVEL_NAME.load(Ordering::Relaxed);
    if level == 0 || unsafe { strike_group() }.is_none() {
        return stock;
    }
    let name = unsafe { CStr::from_ptr(level as *const c_char) };
    let name = name.to_string_lossy();
    if !level_exists(&name) {
        log::warn!(
            "hf_combat_sg: strike group engaged but Data/Levels/{name} is missing; keeping Combat.seria"
        );
        return stock;
    }
    log::info!("hf_combat_sg: strike group engaged, loading {name} instead of Combat.seria");
    level
}

#[no_mangle]
unsafe extern "C" fn version(version: *const c_char) -> bool {
    // SAFETY: the loader passes a live NUL-terminated version string.
    let supported =
        !version.is_null() && unsafe { CStr::from_ptr(version) }.to_bytes() == b"Steam 1.163";
    SUPPORTED_VERSION.store(supported, Ordering::Relaxed);
    supported
}

/// The pointer the replaced `lea rcx,[...]` would have loaded: `LOAD_SITE + 7 + rel32`. Decoded
/// from the verified instruction bytes instead of hard-coding the string address.
fn stock_name_from_site() -> usize {
    let displacement = i32::from_le_bytes([
        LOAD_SITE_BYTES[3],
        LOAD_SITE_BYTES[4],
        LOAD_SITE_BYTES[5],
        LOAD_SITE_BYTES[6],
    ]);
    (LOAD_SITE as i64 + LOAD_SITE_BYTES.len() as i64 + displacement as i64) as usize
}

/// The replacement for `lea rcx,[Combat.seria]`: ask the mod which name to use, then run the
/// original `call SetLevelSeriaToLoad_` with it. RCX is always written, never inherited.
fn trampoline() -> Trampoline {
    let mut trampoline = Trampoline::new();
    // RCX/RDX/R8 and XMM0-5 are restored by the call wrapper; RAX keeps our answer.
    trampoline.preserved_call(level_name as *const (), &[], &[], ReturnType::Rax);
    trampoline.bytes(&[0x48, 0x89, 0xc1]); // mov rcx,rax -- always, the replaced lea never ran
    trampoline.relative_jump(RESUME);
    trampoline
}

/// The site's bytes, once the executable's code is decrypted and readable.
///
/// The Steam build unpacks itself while the modloader starts, so the first check can land on
/// still-encrypted or not yet committed memory; wait for it instead of giving up.
fn wait_for_site() -> Result<(), String> {
    for attempt in 0..=20 {
        if readable(LOAD_SITE, LOAD_SITE_BYTES.len()) {
            // SAFETY: just proved the range readable.
            let current = unsafe {
                std::slice::from_raw_parts(LOAD_SITE as *const u8, LOAD_SITE_BYTES.len())
            };
            if current == LOAD_SITE_BYTES {
                if attempt > 0 {
                    log::info!("hf_combat_sg: hook bytes matched after {attempt} retr(ies)");
                }
                return Ok(());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(format!(
        "code at {LOAD_SITE:#x} differs from the Steam 1.163 build; not patched"
    ))
}

unsafe fn install() -> Result<(), String> {
    wait_for_site()?;
    let stock = stock_name_from_site();
    STOCK_NAME.store(stock, Ordering::Relaxed);
    log::info!("hf_combat_sg: stock level name literal at {stock:#x}");
    let trampoline = trampoline();

    let mut session = PatchSession::new();
    // SAFETY: the site's bytes were verified above; the session is installed permanently in the same call.
    unsafe {
        session
            .detour_trampoline(LOAD_SITE, LOAD_SITE_BYTES.len(), trampoline)
            .map_err(|error| error.to_string())?;
        session.install_permanently().map_err(|error| error.to_string())
    }
}

#[no_mangle]
unsafe extern "C" fn init() -> bool {
    let config = config::load();
    VERBOSE.store(config.verbose_logging, Ordering::Relaxed);
    // SAFETY: a null module name asks for the main module.
    let image_base = unsafe { GetModuleHandleA(std::ptr::null()) } as usize;
    let supported = SUPPORTED_VERSION.load(Ordering::Relaxed);
    log::info!(
        "hf_combat_sg v{}: level {:?}, image base {image_base:#x}, version {}",
        env!("CARGO_PKG_VERSION"),
        config.level,
        if supported { "Steam 1.163" } else { "unsupported" }
    );
    if !supported {
        log::error!("hf_combat_sg: needs Steam 1.163; not started");
        return false;
    }
    if image_base != IMAGE_BASE {
        log::error!(
            "hf_combat_sg: unexpected image base {image_base:#x} (want {IMAGE_BASE:#x}); not started"
        );
        return false;
    }
    if !config.enabled {
        log::info!("hf_combat_sg: disabled in {}", config::PATH);
        return true;
    }
    if INSTALLED.load(Ordering::Relaxed) {
        return true;
    }

    let level = config.level.trim();
    if level.is_empty() || level.contains(['/', '\\']) || !level.ends_with(".seria") {
        log::error!(
            "hf_combat_sg: level {level:?} must be a bare *.seria name in Data/Levels; not started"
        );
        return false;
    }
    let mut bytes = level.as_bytes().to_vec();
    bytes.push(0);
    // Leaked on purpose: the game keeps the pointer for the rest of the process.
    let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    LEVEL_NAME.store(leaked.as_ptr() as usize, Ordering::Relaxed);

    match unsafe { install() } {
        Ok(()) => {
            INSTALLED.store(true, Ordering::Relaxed);
            if !level_exists(level) {
                log::warn!(
                    "hf_combat_sg: Data/Levels/{level} does not exist yet - copy Data/Levels/Combat.seria to it, or strike-group battles keep using Combat.seria"
                );
            }
            log::info!(
                "hf_combat_sg v{}: hooked at {LOAD_SITE:#x}; strike-group battles load {level}",
                env!("CARGO_PKG_VERSION")
            );
            true
        }
        Err(error) => {
            log::error!("hf_combat_sg: not installed: {error}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The generated trampoline must always write RCX (the replaced `lea` never runs) and land
    /// exactly on the original `call SetLevelSeriaToLoad_`.
    #[test]
    fn trampoline_writes_rcx_and_resumes_at_the_call() {
        const ADDRESS: usize = 0x1_7000_0000;
        let code = trampoline().build(ADDRESS).expect("trampoline builds");

        // preserved_call's frame: push rax, rcx, rdx, r8-r11.
        assert_eq!(&code[..7], &[0x50, 0x51, 0x52, 0x41, 0x50, 0x41, 0x51, 0x41][..7]);

        // No conditional: a path that keeps RCX as the caller left it crashes the game.
        assert!(!code.windows(2).any(|window| window == &[0x74u8, 0x03][..]));

        let tail: [u8; 4] = [0x48, 0x89, 0xc1, 0xe9];
        let matches: Vec<usize> = code
            .windows(tail.len())
            .enumerate()
            .filter_map(|(index, window)| (window == tail).then_some(index))
            .collect();
        assert_eq!(matches.len(), 1, "one `mov rcx,rax; jmp` tail");
        let tail_at = matches[0];

        let displacement = i32::from_le_bytes([
            code[tail_at + 4],
            code[tail_at + 5],
            code[tail_at + 6],
            code[tail_at + 7],
        ]);
        let jmp_address = ADDRESS + tail_at + 3;
        assert_eq!(jmp_address as i64 + 5 + displacement as i64, RESUME as i64);
    }

    /// The stock pointer is decoded from the hooked instruction, so it cannot drift from the build.
    #[test]
    fn stock_name_is_decoded_from_the_site() {
        assert_eq!(stock_name_from_site(), 0x14035fa00);
    }

    /// The scan walks the 0x18-stride Link vector and reads the behaviour word of the fleet.
    #[test]
    fn strike_group_scan_reads_the_link_table() {
        let mut fleet = vec![0u8; 0x400];
        fleet[FLEET_BEHAVIOUR..FLEET_BEHAVIOUR + 4].copy_from_slice(&STRIKE_GROUP.to_le_bytes());
        let fleet_address = fleet.as_ptr() as usize;

        let mut ordinary = vec![0u8; 0x400];
        ordinary[FLEET_BEHAVIOUR..FLEET_BEHAVIOUR + 4].copy_from_slice(&2i32.to_le_bytes());
        let ordinary_address = ordinary.as_ptr() as usize;

        // Two links, stride 0x18, object at +8.
        let mut links = vec![0u64; 2 * 3];
        links[1] = ordinary_address as u64;
        links[4] = fleet_address as u64;
        let begin = links.as_ptr() as usize;
        let end = begin + 2 * LINK_STRIDE;

        // SAFETY: the three heap buffers outlive the calls and are fully readable.
        unsafe {
            assert_eq!(strike_group_in(begin, begin), None);
            assert_eq!(strike_group_in(begin + LINK_STRIDE, end), Some(fleet_address));
            assert_eq!(strike_group_in(begin, end), Some(fleet_address));
            assert_eq!(strike_group_in(begin, begin + LINK_STRIDE), None);
            // nonsense bounds never read memory
            assert_eq!(strike_group_in(0, end), None);
            assert_eq!(strike_group_in(end, begin), None);
            assert_eq!(strike_group_in(begin, begin + MAX_LINKS * LINK_STRIDE + LINK_STRIDE), None);
        }
    }

    /// The shipped config template must parse with this build.
    #[test]
    fn shipped_config_template_parses() {
        let config: config::Config =
            serde_json::from_str(include_str!("../release/Modloader/config/combat_sg.json"))
                .expect("template parses");
        assert!(config.enabled);
        assert_eq!(config.level, "Combat_SG.seria");
        assert!(!config.verbose_logging);
    }
}