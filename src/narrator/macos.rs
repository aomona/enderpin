#![allow(unsafe_code)]
//! Trusted-host speech. Minecraft only sends bounded text over its existing stdout pipe.
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::time::Duration;

use objc2::AnyThread;
use objc2::rc::autoreleasepool;
use objc2_avf_audio::{AVSpeechBoundary, AVSpeechSynthesizer, AVSpeechUtterance};
use objc2_foundation::{NSDate, NSRunLoop, NSString};

use crate::narrator::{NarratorCommand, QUEUE_CAPACITY};

pub(crate) fn run_worker(
    receiver: Receiver<NarratorCommand>,
    ready: SyncSender<Result<(), String>>,
) {
    autoreleasepool(|_| {
        // SAFETY: All synthesizer access stays on this worker.
        let voice = unsafe { AVSpeechSynthesizer::new() };
        if ready.send(Ok(())).is_err() {
            return;
        }
        let mut pending = VecDeque::new();
        let run_loop = NSRunLoop::currentRunLoop();
        loop {
            let command = match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            autoreleasepool(|_| {
                // SAFETY: The voice and utterances are used only on their owning worker; volume
                // and text have already passed the shared protocol's bounds and validation.
                unsafe {
                    match command {
                        Some(NarratorCommand::Clear) => {
                            pending.clear();
                            stop(&voice);
                        }
                        Some(NarratorCommand::Say {
                            text,
                            interrupt,
                            volume,
                        }) => {
                            if interrupt {
                                pending.clear();
                                stop(&voice);
                            }
                            if pending.len() < QUEUE_CAPACITY {
                                pending.push_back((text, volume));
                            }
                        }
                        None => {}
                    }
                    // Keep the native synthesizer queue bounded too, not just the IPC channel.
                    if !voice.isSpeaking()
                        && let Some((text, volume)) = pending.pop_front()
                    {
                        let utterance = AVSpeechUtterance::initWithString(
                            AVSpeechUtterance::alloc(),
                            &NSString::from_str(&text),
                        );
                        utterance.setVolume(volume);
                        voice.speakUtterance(&utterance);
                    }
                }
                run_loop.runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
            });
        }
        stop(&voice);
    });
}

fn stop(voice: &AVSpeechSynthesizer) {
    // SAFETY: Called only on the worker owning the synthesizer.
    unsafe {
        voice.stopSpeakingAtBoundary(AVSpeechBoundary::Immediate);
    }
}
