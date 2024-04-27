#![no_std]
#![no_main]

mod swapchain;
mod framebuffer;

use embedded_hal::{digital::OutputPin, spi::SpiBus};
use embedded_hal_0_2::timer::CountDown;
use fugit::{ExtU32, ExtU32Ceil, RateExtU32};
use panic_halt as _;
use swapchain::Swapchain;

use core::cell::RefCell;
use critical_section::Mutex;

use waveshare_rp2040_zero::{hal::{self as hal, clocks::ClocksManager, dma::{single_buffer, DMAExt, SingleChannel}, gpio::{bank0::*, FunctionSioOutput, FunctionSpi, Pin, PinId, PinState, PullDown, PullNone, SioOutput}, resets, spi::FrameFormat, timer::{Alarm, Alarm0}, Clock, Spi, Timer}, XOSC_CRYSTAL_FREQ};
use hal::pac;
use pac::interrupt;

const DISPLAY_HEIGHT: usize = 256;

type SpiPins = (Pin<Gpio3, FunctionSpi, PullDown>, Pin<Gpio2, FunctionSpi, PullDown>);
type SpiPinsIdle = (Pin<Gpio3, FunctionSpi, PullDown>, Pin<Gpio2, FunctionSioOutput, PullDown>);
type SpiParts = (SpiPinsIdle, pac::SPI0);
type DisplaySpi = Spi<hal::spi::Enabled, pac::SPI0, SpiPins, 8>;
type DisplayDma = hal::dma::Channel<hal::dma::CH0>;
type DisplayDmaTransfer = hal::dma::single_buffer::Transfer<DisplayDma, &'static [u8], DisplaySpi>;

fn parts_to_spi(parts: SpiParts, resets: &mut pac::RESETS, clocks: &ClocksManager) -> DisplaySpi {
    Spi::<_, _, _, 8>::new(parts.1, (parts.0.0, parts.0.1.into_function()))
        .init(resets, clocks.peripheral_clock.freq(), 20_000_000u32.Hz(), FrameFormat::MotorolaSpi(embedded_hal::spi::MODE_3))
}

fn spi_to_parts(spi: DisplaySpi) -> SpiParts {
    let spi = spi.disable();
    let (spi0, (tx, ck)) = spi.free();
    ((tx, ck.into_push_pull_output_in_state(PinState::Low)), spi0)
}

struct DisplayHw {
    hs_pin: Pin<Gpio0, FunctionSioOutput, PullDown>,
    vs_pin: Pin<Gpio1, FunctionSioOutput, PullDown>,
    timer: Timer,
    alarm: Alarm0,
    spi_parts: Option<SpiParts>,
    dma: Option<DisplayDma>,
    transfer: Option<DisplayDmaTransfer>,
    row: usize,
    resets: pac::RESETS,
    clocks: ClocksManager,
}

static DISPLAY_HW: Mutex<RefCell<Option<DisplayHw>>> = Mutex::new(RefCell::new(None));
static SWAPCHAIN: Swapchain = Swapchain::new();

#[hal::entry]
fn main() -> ! {
    let mut pac = pac::Peripherals::take().unwrap();
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let sio = hal::Sio::new(pac.SIO);

    let clocks = hal::clocks::init_clocks_and_plls(
        XOSC_CRYSTAL_FREQ,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .unwrap();

    let pins = hal::gpio::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    let mut hs_pin = pins.gpio0.into_push_pull_output_in_state(hal::gpio::PinState::High);
    let mut vs_pin = pins.gpio1.into_push_pull_output_in_state(hal::gpio::PinState::Low);
    hs_pin.set_drive_strength(hal::gpio::OutputDriveStrength::FourMilliAmps);
    vs_pin.set_drive_strength(hal::gpio::OutputDriveStrength::FourMilliAmps);
    hs_pin.set_slew_rate(hal::gpio::OutputSlewRate::Slow);
    vs_pin.set_slew_rate(hal::gpio::OutputSlewRate::Slow);

    let _en_pin = pins.gpio4.into_push_pull_output_in_state(PinState::High);

    let mut ck_pin = pins.gpio2.into_push_pull_output_in_state(PinState::Low);
    let mut tx_pin = pins.gpio3.into_function::<FunctionSpi>();
    ck_pin.set_drive_strength(hal::gpio::OutputDriveStrength::FourMilliAmps);
    tx_pin.set_drive_strength(hal::gpio::OutputDriveStrength::FourMilliAmps);
    ck_pin.set_slew_rate(hal::gpio::OutputSlewRate::Slow);
    tx_pin.set_slew_rate(hal::gpio::OutputSlewRate::Slow);

    let spi_parts = ((tx_pin, ck_pin), pac.SPI0);

    let mut dma = pac.DMA.split(&mut pac.RESETS).ch0;

    let mut timer = Timer::new(pac.TIMER, &mut pac.RESETS, &clocks);
    let mut alarm = timer.alarm_0().unwrap();

    critical_section::with(move |cs| {
        _ = alarm.schedule(100u32.millis());
        
        dma.enable_irq0();
        alarm.enable_interrupt();

        DISPLAY_HW.borrow_ref_mut(cs).replace(DisplayHw {
            timer,
            hs_pin,
            vs_pin,
            alarm,
            spi_parts: Some(spi_parts),
            dma: Some(dma),
            transfer: None,
            row: 0,
            resets: pac.RESETS,
            clocks: clocks
        });
    });

    unsafe {
        pac::NVIC::unmask(pac::Interrupt::TIMER_IRQ_0);
        pac::NVIC::unmask(pac::Interrupt::DMA_IRQ_0);
    }

    loop {
        let mut image = SWAPCHAIN.acquire_next();
        let fb = image.framebuffer();
        fb.clear(false);
        for y in 0..100 {
            for x in 0..y {
                fb.set_pixel(x, y, true);
            }
        }
        image.submit();

    }
}

#[interrupt]
fn DMA_IRQ_0() {
    pac::NVIC::mask(pac::Interrupt::TIMER_IRQ_0);
    critical_section::with(|cs| {
        let mut display_hw_borrow = DISPLAY_HW.borrow_ref_mut(cs);
        if let Some(mut display_hw) = display_hw_borrow.take() {
            let transfer = display_hw.transfer.take().unwrap();
            let (mut dma, _, spi) = transfer.wait();

            dma.check_irq0();

            let spi_parts = spi_to_parts(spi);

            if display_hw.row == 0 {
                _ = display_hw.vs_pin.set_low();
            }

            display_hw.dma = Some(dma);
            display_hw.spi_parts = Some(spi_parts);

            display_hw_borrow.replace(display_hw);
        }
    });
    unsafe { pac::NVIC::unmask(pac::Interrupt::TIMER_IRQ_0); }
}

#[interrupt]
fn TIMER_IRQ_0() {
    pac::NVIC::mask(pac::Interrupt::DMA_IRQ_0);
    critical_section::with(|cs| {
        let mut display_hw_borrow = DISPLAY_HW.borrow_ref_mut(cs);
        if let Some(mut display_hw) = display_hw_borrow.take() {

            _ = display_hw.alarm.cancel();
            _ = display_hw.alarm.clear_interrupt();
            _ = display_hw.alarm.schedule(60.micros());

            let mut countdown = display_hw.timer.count_down();

            match (display_hw.spi_parts.take(), display_hw.dma.take()) {
                (Some(mut spi_parts), Some(mut dma)) => {
                    let fb = SWAPCHAIN.read();
                    let row = display_hw.row;
                    display_hw.row += 1;
                    display_hw.row %= DISPLAY_HEIGHT + 1;
                    
                    _ = display_hw.hs_pin.set_low();
                    {
                        let ck_pin = &mut spi_parts.0.1;

                        for _ in 0..4 {
                            _ = ck_pin.set_low();

                            countdown.start(100.nanos_at_least());
                            _ = nb::block!(countdown.wait());

                            _ = ck_pin.set_high();

                            countdown.start(100.nanos_at_least());
                            _ = nb::block!(countdown.wait());
                        }

                        _ = display_hw.hs_pin.set_high();
                    };

                    if row == 0 {
                        _ = display_hw.vs_pin.set_high();
                    }

                    if row < DISPLAY_HEIGHT {
                        dma.enable_irq0();
                        let spi = parts_to_spi(spi_parts, &mut display_hw.resets, &display_hw.clocks);
                        let transfer = single_buffer::Config::new(dma, fb.row_slice(row), spi).start();
                        display_hw.transfer = Some(transfer);
                    } else {
                        display_hw.spi_parts = Some(spi_parts);
                        display_hw.dma = Some(dma);
                    }

                    if row == 0 {
                        countdown.start(1.micros_at_least());
                        _ = nb::block!(countdown.wait());
                        _ = display_hw.vs_pin.set_low();
                        countdown.start(3.micros_at_least());
                        _ = display_hw.vs_pin.set_high();
                    }
                },
                (Some(spi_parts), None) => { display_hw.spi_parts = Some(spi_parts) },
                (None, Some(dma)) => { display_hw.dma = Some(dma) },
                _ => {}
            }
            display_hw.alarm.enable_interrupt();
            display_hw_borrow.replace(display_hw);
        }
    });
    unsafe { pac::NVIC::unmask(pac::Interrupt::DMA_IRQ_0); }
}
