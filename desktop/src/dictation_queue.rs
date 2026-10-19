//! Keep accepted audio in order while one recognition/editor job is running.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const MERGE_SECONDS: f64 = 25.;
const BACKLOG_SECONDS: f64 = 45.;

pub struct AudioSegment {
    pub samples: Vec<f32>,
    pub rate: u32,
    pub queued_at: Instant,
}
impl AudioSegment {
    fn seconds(&self) -> f64 {
        self.samples.len() as f64 / self.rate as f64
    }
}

#[derive(Default)]
pub struct AudioQueue {
    segments: VecDeque<AudioSegment>,
}
impl AudioQueue {
    /// The threshold requests a capture stop, never rejects accepted audio. A
    /// stop flush may still append the final (at most 25-second) capture segment.
    pub fn push(&mut self, segment: AudioSegment, merge: bool) -> bool {
        if let Some(last) = self.segments.back_mut().filter(|last| {
            merge
                && last.rate == segment.rate
                && last.seconds() + segment.seconds() <= MERGE_SECONDS
        }) {
            last.samples.extend(segment.samples);
        } else {
            self.segments.push_back(segment);
        }
        self.should_pause(Instant::now())
    }
    pub fn pop(&mut self) -> Option<AudioSegment> {
        self.segments.pop_front()
    }
    pub fn restore_front(&mut self, segment: AudioSegment) {
        self.segments.push_front(segment);
    }
    pub fn clear(&mut self) {
        self.segments.clear();
    }
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
    pub fn seconds(&self) -> f64 {
        self.segments.iter().map(AudioSegment::seconds).sum()
    }
    pub fn should_pause(&self, now: Instant) -> bool {
        self.seconds() >= BACKLOG_SECONDS
            || self.segments.front().is_some_and(|first| {
                now.saturating_duration_since(first.queued_at) >= Duration::from_secs(45)
            })
    }
}

/// Once a continuous session loses its destination, later focus changes cannot
/// re-enable insertion of old speech. New recording sessions get a new guard.
#[derive(Default)]
pub struct DeliveryGuard {
    blocked: bool,
}
impl DeliveryGuard {
    pub fn observe(&mut self, target_matches: bool) -> bool {
        let changed = !self.blocked && !target_matches;
        if changed {
            self.blocked = true;
        }
        changed
    }
    pub fn block(&mut self) {
        self.blocked = true;
    }
    pub fn allowed(&self) -> bool {
        !self.blocked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(seconds: usize, value: f32) -> AudioSegment {
        AudioSegment {
            samples: vec![value; seconds * 100],
            rate: 100,
            queued_at: Instant::now(),
        }
    }

    #[test]
    fn waiting_speech_merges_without_reordering_or_extending_the_first_queue_time() {
        let mut queue = AudioQueue::default();
        let first = segment(10, 1.);
        let first_at = first.queued_at;
        queue.push(first, true);
        queue.push(segment(12, 2.), true);
        queue.push(segment(5, 3.), true);
        let first = queue.pop().unwrap();
        assert_eq!(first.queued_at, first_at);
        assert_eq!(first.samples[..1000], vec![1.; 1000]);
        assert_eq!(first.samples[1000..], vec![2.; 1200]);
        assert_eq!(queue.pop().unwrap().samples, vec![3.; 500]);
        assert!(queue.is_empty());
    }

    #[test]
    fn backlog_stop_preserves_threshold_segment_and_capture_tail() {
        let mut queue = AudioQueue::default();
        assert!(!queue.push(segment(25, 1.), true));
        assert!(queue.push(segment(25, 2.), true));
        assert!(queue.push(segment(3, 3.), true));
        assert_eq!(queue.seconds(), 53.);
        for (seconds, value) in [(25, 1.), (25, 2.), (3, 3.)] {
            assert_eq!(queue.pop().unwrap().samples, vec![value; seconds * 100]);
        }
    }

    #[test]
    fn worker_backpressure_preserves_front_and_cancel_clears_accepted_work() {
        let mut queue = AudioQueue::default();
        queue.push(segment(2, 1.), false);
        queue.push(segment(2, 2.), false);
        let waiting = queue.pop().unwrap();
        queue.restore_front(waiting);
        assert_eq!(queue.pop().unwrap().samples, vec![1.; 200]);
        queue.clear();
        assert!(queue.pop().is_none());
        assert_eq!(queue.seconds(), 0.);
    }

    #[test]
    fn short_utterances_also_stop_capture_when_waiting_too_long() {
        let mut queue = AudioQueue::default();
        let first = segment(1, 1.);
        let started = first.queued_at;
        queue.push(first, true);
        assert!(!queue.should_pause(started + Duration::from_secs(44)));
        assert!(queue.should_pause(started + Duration::from_secs(45)));
        assert_eq!(queue.pop().unwrap().samples.len(), 100);
    }

    #[test]
    fn returning_focus_does_not_deliver_speech_from_before_the_switch() {
        let mut guard = DeliveryGuard::default();
        assert!(!guard.observe(true));
        assert!(guard.allowed());
        assert!(guard.observe(false));
        assert!(!guard.observe(true));
        assert!(!guard.allowed());
        assert!(DeliveryGuard::default().allowed());
    }

    #[test]
    fn failed_segment_prevents_later_segments_from_being_inserted_as_complete_text() {
        let mut guard = DeliveryGuard::default();
        guard.block();
        assert!(!guard.observe(true));
        assert!(!guard.allowed());
    }
}
