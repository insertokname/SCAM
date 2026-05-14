#![cfg(test)]

mod test_logger;

use std::env;

use crate::{devices::nes::Nes, hardware::cartrige::Cartrige, test::test_logger::TestLogger};

static NESTEST_TEST_LOGGER: TestLogger = TestLogger::new();

#[test]
fn nestest() {
    log::set_logger(&NESTEST_TEST_LOGGER).unwrap();
    log::set_max_level(log::LevelFilter::Trace);

    let mut nes = Nes::new();
    // Thank you nes dev wiki https://www.nesdev.org/wiki/Emulator_tests
    let cartrige = Cartrige::from_bytes(include_bytes!("./nestest/nestest.nes")).unwrap();

    nes.insert_cartrige(cartrige);
    nes.reset_with_program_counter(0xC000);

    loop {
        nes.tick();
        if nes.is_resetting() {
            println!("cpu has been reset");
            break;
        }
    }

    let correct_log = include_str!("./nestest/correct_nestest.log");
    // windows line endings lmfao
    let correct_log = correct_log.replace("\r\n", "\n");
    let actual_log_res = NESTEST_TEST_LOGGER.logs.read().unwrap();
    let actual_log = actual_log_res.as_str();

    let equal = correct_log == actual_log;
    if !equal {
        let mut actual_log_path = env::temp_dir();
        actual_log_path.push("scam_nestest_actual.log");
        std::fs::write(&actual_log_path, actual_log).unwrap();

        let mut correct_log_path = env::temp_dir();
        correct_log_path.push("scam_nestest_correct.log");
        std::fs::write(&correct_log_path, correct_log).unwrap();

        println!("Nestest failed! Make sure cpu opcodes are sane.");
        println!("Got logs: {}", actual_log_path.display());
        println!(
            "Instead should have been like these: {}",
            correct_log_path.display()
        );
        panic!()
    }
}
