mod common;
mod debugger;
mod gdb;
mod info;
mod run;

use debugger::CliState;

use probe_rs::{
    debug::debug_info::DebugInfo,
    flashing::{erase_all, BinOptions, FileDownloadError, Format},
    MemoryInterface, Probe,
};

use probe_rs_cli_util::{
    clap,
    clap::Parser,
    common_options::{print_chip_info, print_families, CargoOptions, FlashOptions, ProbeOptions},
    flash::run_flash_download,
};

use rustyline::Editor;

use anyhow::{Context, Result};

use std::{fs::File, path::PathBuf};
use std::{io, time::Instant};
use std::{num::ParseIntError, path::Path};

#[derive(clap::Parser)]
#[clap(
    name = "probe-rs CLI",
    about = "A CLI for on top of the debug probe capabilities provided by probe-rs",
    author = "Noah Hüsser <yatekii@yatekii.ch> / Dominik Böhi <dominik.boehi@gmail.ch>"
)]
enum Cli {
    /// List all connected debug probes
    List {},
    /// Gets infos about the selected debug probe and connected target
    Info {
        #[clap(flatten)]
        common: ProbeOptions,
    },
    /// Resets the target attached to the selected debug probe
    Reset {
        #[clap(flatten)]
        shared: CoreOptions,

        #[clap(flatten)]
        common: ProbeOptions,

        /// Whether the reset pin should be asserted or deasserted. If left open, just pulse it
        assert: Option<bool>,
    },
    /// Run a GDB server
    Gdb {
        #[clap(
            long,
            help = "Use this flag to override the default GDB connection string (localhost:1337)."
        )]
        gdb_connection_string: Option<String>,

        #[structopt(
            name = "reset-halt",
            long = "reset-halt",
            help = "Use this flag to reset and halt (instead of just a halt) the attached core after attaching to the target."
        )]
        reset_halt: bool,
        #[clap(flatten)]
        common: ProbeOptions,
    },
    /// Basic command line debugger
    Debug {
        #[clap(flatten)]
        shared: CoreOptions,

        #[clap(flatten)]
        common: ProbeOptions,

        #[structopt(long, parse(from_os_str))]
        /// Binary to debug
        exe: Option<PathBuf>,
    },
    /// Dump memory from attached target
    Dump {
        #[structopt(flatten)]
        shared: CoreOptions,

        #[structopt(flatten)]
        common: ProbeOptions,

        /// The address of the memory to dump from the target.
        #[structopt(parse(try_from_str = parse_u32))]
        loc: u32,
        /// The amount of memory (in words) to dump.
        #[structopt(parse(try_from_str = parse_u32))]
        words: u32,
    },
    /// Download memory to attached target
    Download {
        #[structopt(flatten)]
        common: ProbeOptions,

        /// Format of the file to be downloaded to the flash. Possible values are case-insensitive.
        #[clap(arg_enum, ignore_case = true, default_value = "elf", long)]
        format: DownloadFileType,

        /// The address in memory where the binary will be put at. This is only considered when `bin` is selected as the format.
        #[structopt(long, parse(try_from_str = parse_u32))]
        base_address: Option<u32>,
        /// The number of bytes to skip at the start of the binary file. This is only considered when `bin` is selected as the format.
        #[structopt(long, parse(try_from_str = parse_u32))]
        skip_bytes: Option<u32>,

        /// The path to the file to be downloaded to the flash
        path: String,

        /// Whether to erase the entire chip before downloading
        #[structopt(long)]
        chip_erase: bool,

        /// Whether to disable fancy progress reporting
        #[structopt(long)]
        disable_progressbars: bool,

        /// Disable double-buffering when downloading flash.  If downloading times out, try this option.
        #[structopt(long = "disable-double-buffering")]
        disable_double_buffering: bool,
    },
    /// Erase all nonvolatile memory of attached target
    Erase {
        #[structopt(flatten)]
        common: ProbeOptions,
    },
    /// Flash and run an ELF program
    #[structopt(name = "run")]
    Run {
        #[structopt(flatten)]
        common: ProbeOptions,

        /// The path to the ELF file to flash and run
        path: String,

        /// Whether to erase the entire chip before downloading
        #[structopt(long)]
        chip_erase: bool,

        /// Disable double-buffering when downloading flash.  If downloading times out, try this option.
        #[structopt(long = "disable-double-buffering")]
        disable_double_buffering: bool,
    },
    /// Trace a memory location on the target
    #[structopt(name = "trace")]
    Trace {
        #[structopt(flatten)]
        shared: CoreOptions,

        #[structopt(flatten)]
        common: ProbeOptions,

        /// The address of the memory to dump from the target.
        #[structopt(parse(try_from_str = parse_u32))]
        loc: u32,
    },
    #[clap(subcommand)]
    Chip(Chip),
}

#[derive(clap::StructOpt)]
/// Inspect internal registry of supported chips
enum Chip {
    /// Lists all the available families and their chips with their full.
    #[structopt(name = "list")]
    List,
    /// Shows chip properties of a specific chip
    #[structopt(name = "info")]
    Info {
        /// The name of the chip to display.
        name: String,
    },
}

/// Shared options for core selection, shared between commands
#[derive(clap::StructOpt)]
struct CoreOptions {
    #[structopt(long, default_value = "0")]
    core: usize,
}

fn main() -> Result<()> {
    // Initialize the logging backend.
    pretty_env_logger::init();

    let matches = Cli::parse();

    match matches {
        Cli::List {} => list_connected_devices(),
        Cli::Info { common } => crate::info::show_info_of_device(&common),
        Cli::Gdb {
            gdb_connection_string,
            common,
            reset_halt,
        } => gdb::run_gdb_server(common, gdb_connection_string.as_deref(), reset_halt),
        Cli::Reset {
            shared,
            common,
            assert,
        } => reset_target_of_device(&shared, &common, assert),
        Cli::Debug {
            shared,
            common,
            exe,
        } => debug(&shared, &common, exe),
        Cli::Dump {
            shared,
            common,
            loc,
            words,
        } => dump_memory(&shared, &common, loc, words),
        Cli::Download {
            common,
            format,
            base_address,
            skip_bytes,
            path,
            chip_erase,
            disable_progressbars,
            disable_double_buffering,
        } => download_program_fast(
            common,
            format.into(base_address, skip_bytes),
            &path,
            chip_erase,
            disable_progressbars,
            disable_double_buffering,
        ),
        Cli::Run {
            common,
            path,
            chip_erase,
            disable_double_buffering,
        } => run::run(common, &path, chip_erase, disable_double_buffering),
        Cli::Erase { common } => erase(&common),
        Cli::Trace {
            shared,
            common,
            loc,
        } => trace_u32_on_target(&shared, &common, loc),
        Cli::Chip(Chip::List) => print_families(io::stdout()).map_err(Into::into),
        Cli::Chip(Chip::Info { name }) => print_chip_info(name, io::stdout()),
    }
}

fn list_connected_devices() -> Result<()> {
    let links = Probe::list_all();

    if !links.is_empty() {
        println!("The following devices were found:");
        links
            .iter()
            .enumerate()
            .for_each(|(num, link)| println!("[{}]: {:?}", num, link));
    } else {
        println!("No devices were found.");
    }

    Ok(())
}

fn dump_memory(
    shared_options: &CoreOptions,
    common: &ProbeOptions,
    loc: u32,
    words: u32,
) -> Result<()> {
    let mut session = common.simple_attach()?;

    let mut data = vec![0_u32; words as usize];

    // Start timer.
    let instant = Instant::now();

    // let loc = 220 * 1024;

    let mut core = session.core(shared_options.core)?;

    core.read_32(loc, data.as_mut_slice())?;
    // Stop timer.
    let elapsed = instant.elapsed();

    // Print read values.
    for word in 0..words {
        println!(
            "Addr 0x{:08x?}: 0x{:08x}",
            loc + 4 * word,
            data[word as usize]
        );
    }
    // Print stats.
    println!("Read {:?} words in {:?}", words, elapsed);

    Ok(())
}

fn download_program_fast(
    common: ProbeOptions,
    format: Format,
    path: &str,
    do_chip_erase: bool,
    disable_progressbars: bool,
    disable_double_buffering: bool,
) -> Result<()> {
    let mut session = common.simple_attach()?;

    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) => return Err(FileDownloadError::IO(e)).context("Failed to open binary file."),
    };

    let mut loader = session.target().flash_loader();

    match format {
        Format::Bin(options) => loader.load_bin_data(&mut file, options),
        Format::Elf => loader.load_elf_data(&mut file),
        Format::Hex => loader.load_hex_data(&mut file),
    }?;

    run_flash_download(
        &mut session,
        Path::new(path),
        &FlashOptions {
            version: false,
            list_chips: false,
            list_probes: false,
            disable_progressbars,
            disable_double_buffering,
            reset_halt: false,
            log: None,
            restore_unwritten: false,
            flash_layout_output_path: None,
            elf: None,
            work_dir: None,
            cargo_options: CargoOptions::default(),
            probe_options: common,
        },
        loader,
        do_chip_erase,
    )?;

    Ok(())
}

fn erase(common: &ProbeOptions) -> Result<()> {
    let mut session = common.simple_attach()?;

    erase_all(&mut session)?;

    Ok(())
}

fn reset_target_of_device(
    shared_options: &CoreOptions,
    common: &ProbeOptions,
    _assert: Option<bool>,
) -> Result<()> {
    let mut session = common.simple_attach()?;

    session.core(shared_options.core)?.reset()?;

    Ok(())
}

fn trace_u32_on_target(
    shared_options: &CoreOptions,
    common: &ProbeOptions,
    loc: u32,
) -> Result<()> {
    use scroll::{Pwrite, LE};
    use std::io::prelude::*;
    use std::thread::sleep;
    use std::time::Duration;

    let mut xs = vec![];
    let mut ys = vec![];

    let start = Instant::now();

    let mut session = common.simple_attach()?;

    let mut core = session.core(shared_options.core)?;

    loop {
        // Prepare read.
        let elapsed = start.elapsed();
        let instant = elapsed.as_secs() * 1000 + u64::from(elapsed.subsec_millis());

        // Read data.
        let value: u32 = core.read_word_32(loc)?;

        xs.push(instant);
        ys.push(value);

        // Send value to plot.py.
        let mut buf = [0_u8; 8];
        // Unwrap is safe!
        buf.pwrite_with(instant, 0, LE).unwrap();
        buf.pwrite_with(value, 4, LE).unwrap();
        std::io::stdout().write_all(&buf)?;

        std::io::stdout().flush()?;

        // Schedule next read.
        let elapsed = start.elapsed();
        let instant = elapsed.as_secs() * 1000 + u64::from(elapsed.subsec_millis());
        let poll_every_ms = 50;
        let time_to_wait = poll_every_ms - instant % poll_every_ms;
        sleep(Duration::from_millis(time_to_wait));
    }
}

fn debug(shared_options: &CoreOptions, common: &ProbeOptions, exe: Option<PathBuf>) -> Result<()> {
    let mut session = common.simple_attach()?;

    let di = exe
        .as_ref()
        .and_then(|path| DebugInfo::from_file(path).ok());

    let cli = debugger::DebugCli::new();

    let core = session.core(shared_options.core)?;

    let mut cli_data = debugger::CliData::new(core, di)?;

    let mut rl = Editor::<()>::new();

    loop {
        cli_data.print_state()?;

        let readline = rl.readline(">> ");
        match readline {
            Ok(line) => {
                let history_entry: &str = line.as_ref();
                rl.add_history_entry(history_entry);
                let cli_state = cli.handle_line(&line, &mut cli_data)?;

                match cli_state {
                    CliState::Continue => (),
                    CliState::Stop => break,
                }
            }
            Err(e) => {
                use rustyline::error::ReadlineError;

                match e {
                    // For end of file and ctrl-c, we just quit
                    ReadlineError::Eof | ReadlineError::Interrupted => return Ok(()),
                    actual_error => {
                        // Show error message and quit
                        println!("Error handling input: {:?}", actual_error);
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

#[derive(clap::ArgEnum, Debug, Clone, Copy)]
enum DownloadFileType {
    Elf,
    Hex,
    Bin,
}

impl DownloadFileType {
    fn into(self, base_address: Option<u32>, skip: Option<u32>) -> Format {
        match self {
            DownloadFileType::Elf => Format::Elf,
            DownloadFileType::Hex => Format::Hex,
            DownloadFileType::Bin => Format::Bin(BinOptions {
                base_address,
                skip: skip.unwrap_or(0),
            }),
        }
    }
}

fn parse_u32(input: &str) -> Result<u32, ParseIntError> {
    parse_int::parse(input)
}
