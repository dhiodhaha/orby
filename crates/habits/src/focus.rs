//! Focus timer (ORB_SPEC 6.4, DESIGN.md "Focus (v6.2)"): a pomodoro state machine.
//!
//! The caller drives it with commands and a regular `tick(now)`. Each call returns an [`Outcome`]
//! listing phase transitions and events (focus done, break done, session logged).

use std::time::Duration;

use crate::time::{mins, Date, Moment};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusConfig {
    pub focus: Duration,
    pub short_break: Duration,
    pub long_break: Duration,
    /// A long break follows every this many finished sessions.
    pub sessions_per_long_break: u32,
    /// Start the next focus session by itself when a break ends.
    pub auto_start_next: bool,
}

impl Default for FocusConfig {
    fn default() -> Self {
        FocusConfig {
            focus: mins(25),
            short_break: mins(5),
            long_break: mins(15),
            sessions_per_long_break: 4,
            auto_start_next: false,
        }
    }
}

/// The task a session is logged against (the tasks module owns the real id).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TaskId(pub String);

/// Session `n` of `of` in the current cycle, for the session dots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    pub n: u32,
    pub of: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PauseReason {
    User,
    /// "Memasuki waktu {prayer}" paused it; it resumes when that pill is dismissed.
    Prayer(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakKind {
    Short,
    Long,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    /// `started_at` is shifted forward by any pause, so `started_at + length` is always the end.
    Focus {
        session: Session,
        started_at: Moment,
        length: Duration,
        task: Option<TaskId>,
    },
    Paused {
        session: Session,
        length: Duration,
        remaining: Duration,
        task: Option<TaskId>,
        reason: PauseReason,
    },
    Break {
        kind: BreakKind,
        started_at: Moment,
        length: Duration,
    },
}

/// A phase without its data, for transitions and errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhaseKind {
    Idle,
    Focus,
    Paused,
    Break(BreakKind),
}

impl Phase {
    pub fn kind(&self) -> PhaseKind {
        match self {
            Phase::Idle => PhaseKind::Idle,
            Phase::Focus { .. } => PhaseKind::Focus,
            Phase::Paused { .. } => PhaseKind::Paused,
            Phase::Break { kind, .. } => PhaseKind::Break(*kind),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transition {
    pub at: Moment,
    pub from: PhaseKind,
    pub to: PhaseKind,
}

/// One finished (or skipped-to-break) session, for History.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRecord {
    pub date: Date,
    pub ended_at: Moment,
    pub minutes: u32,
    pub task: Option<TaskId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FocusEvent {
    /// The timer ran out: "Focus done · 25 min".
    FocusDone {
        session: Session,
        minutes: u32,
    },
    BreakDone {
        kind: BreakKind,
    },
    SessionLogged(SessionRecord),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub transitions: Vec<Transition>,
    pub events: Vec<FocusEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusError {
    /// The command does not apply in this phase (e.g. pause while idle).
    NotAllowed(PhaseKind),
    ZeroLength,
}

#[derive(Clone, Debug)]
pub struct FocusTimer {
    config: FocusConfig,
    phase: Phase,
    /// Sessions finished in the current cycle, 0 to `sessions_per_long_break`.
    done_in_cycle: u32,
    /// Length and task of the last start, reused when the next session starts by itself.
    last_length: Duration,
    last_task: Option<TaskId>,
    log: Vec<SessionRecord>,
}

impl FocusTimer {
    pub fn new(config: FocusConfig) -> Self {
        let last_length = config.focus;
        FocusTimer { config, phase: Phase::Idle, done_in_cycle: 0, last_length, last_task: None, log: Vec::new() }
    }

    pub fn config(&self) -> &FocusConfig {
        &self.config
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Finished sessions in this cycle (filled session dots).
    pub fn sessions_done(&self) -> u32 {
        self.done_in_cycle
    }

    /// Finished sessions, oldest first.
    pub fn log(&self) -> &[SessionRecord] {
        &self.log
    }

    /// True while normal notices should be held: during focus, including a pause the user made.
    /// A prayer pause holds nothing: the user is away praying, with no end in sight, so it counts
    /// as a break for notices (ORB_SPEC 6.4 holds them "during focus" only).
    pub fn is_holding(&self) -> bool {
        matches!(self.phase, Phase::Focus { .. } | Phase::Paused { reason: PauseReason::User, .. })
    }

    /// When the current focus would hand over to its break, if focus is on. Drink reminders wait
    /// for it. A prayer pause has no break to wait for (see [`Self::is_holding`]).
    pub fn break_at(&self, now: Moment) -> Option<Moment> {
        match &self.phase {
            Phase::Focus { started_at, length, .. } => Some(*started_at + *length),
            Phase::Paused { remaining, reason: PauseReason::User, .. } => Some(now + *remaining),
            _ => None,
        }
    }

    /// Time left in the current focus, pause or break.
    pub fn remaining(&self, now: Moment) -> Option<Duration> {
        match &self.phase {
            Phase::Idle => None,
            Phase::Focus { started_at, length, .. } | Phase::Break { started_at, length, .. } => {
                Some((*started_at + *length).since(now))
            }
            Phase::Paused { remaining, .. } => Some(*remaining),
        }
    }

    /// Share of the current phase already spent, 0 to 1, for the ring. Idle is 0.
    pub fn progress(&self, now: Moment) -> f32 {
        let length = match &self.phase {
            Phase::Idle => return 0.0,
            Phase::Focus { length, .. } | Phase::Paused { length, .. } | Phase::Break { length, .. } => *length,
        };
        let left = self.remaining(now).unwrap_or(Duration::ZERO);
        if length.is_zero() {
            return 1.0;
        }
        (1.0 - left.as_secs_f32() / length.as_secs_f32()).clamp(0.0, 1.0)
    }

    /// Start focus from idle, or end a break early. `None` length uses the configured focus length.
    pub fn start(
        &mut self,
        now: Moment,
        length: Option<Duration>,
        task: Option<TaskId>,
    ) -> Result<Outcome, FocusError> {
        if !matches!(self.phase, Phase::Idle | Phase::Break { .. }) {
            return Err(FocusError::NotAllowed(self.phase.kind()));
        }
        let length = length.unwrap_or(self.config.focus);
        if length.is_zero() {
            return Err(FocusError::ZeroLength);
        }
        self.last_length = length;
        self.last_task = task;
        let mut out = Outcome::default();
        self.begin_focus(now, &mut out);
        Ok(out)
    }

    pub fn pause(&mut self, now: Moment) -> Result<Outcome, FocusError> {
        if !matches!(self.phase, Phase::Focus { .. }) {
            return Err(FocusError::NotAllowed(self.phase.kind()));
        }
        Ok(self.pause_with(now, PauseReason::User))
    }

    /// Prayer at 0 s pauses a running session. Outside focus, or when already paused, it does nothing.
    pub fn pause_for_prayer(&mut self, now: Moment, prayer: &str) -> Outcome {
        match self.phase {
            Phase::Focus { .. } => self.pause_with(now, PauseReason::Prayer(prayer.to_string())),
            _ => Outcome::default(),
        }
    }

    pub fn resume(&mut self, now: Moment) -> Result<Outcome, FocusError> {
        let Phase::Paused { session, length, remaining, task, .. } = &self.phase else {
            return Err(FocusError::NotAllowed(self.phase.kind()));
        };
        let started_at = now - (*length - *remaining);
        let next = Phase::Focus { session: *session, started_at, length: *length, task: task.clone() };
        let mut out = Outcome::default();
        self.go(now, next, &mut out);
        Ok(out)
    }

    /// Called when the prayer pill is dismissed: resumes only a session the prayer paused,
    /// never one the user paused.
    pub fn resume_after_prayer(&mut self, now: Moment) -> Outcome {
        match &self.phase {
            Phase::Paused { reason: PauseReason::Prayer(_), .. } => self.resume(now).unwrap_or_default(),
            _ => Outcome::default(),
        }
    }

    /// Ends the run without logging the current session. The next start begins a new cycle.
    pub fn stop(&mut self, now: Moment) -> Result<Outcome, FocusError> {
        if self.phase == Phase::Idle {
            return Err(FocusError::NotAllowed(PhaseKind::Idle));
        }
        self.done_in_cycle = 0;
        let mut out = Outcome::default();
        self.go(now, Phase::Idle, &mut out);
        Ok(out)
    }

    /// Ends focus early and starts the break. The minutes focused so far are logged.
    pub fn skip_to_break(&mut self, now: Moment) -> Result<Outcome, FocusError> {
        let (spent, task) = match &self.phase {
            Phase::Focus { started_at, task, .. } => (now.since(*started_at), task.clone()),
            Phase::Paused { length, remaining, task, .. } => (*length - *remaining, task.clone()),
            _ => return Err(FocusError::NotAllowed(self.phase.kind())),
        };
        let mut out = Outcome::default();
        let minutes = (spent.as_secs() / 60) as u32;
        if minutes > 0 {
            self.record(now, minutes, task, &mut out);
        }
        self.begin_break(now, &mut out);
        Ok(out)
    }

    /// Advances the clock. A late tick catches up through every phase that ended by `now`,
    /// each transition stamped with the moment it was due, not the moment of the tick.
    pub fn tick(&mut self, now: Moment) -> Outcome {
        let mut out = Outcome::default();
        loop {
            match &self.phase {
                Phase::Focus { session, started_at, length, task } if *started_at + *length <= now => {
                    let end = *started_at + *length;
                    let minutes = (length.as_secs() as u32).div_ceil(60);
                    let (session, task) = (*session, task.clone());
                    out.events.push(FocusEvent::FocusDone { session, minutes });
                    self.record(end, minutes, task, &mut out);
                    self.begin_break(end, &mut out);
                }
                Phase::Break { kind, started_at, length } if *started_at + *length <= now => {
                    let end = *started_at + *length;
                    out.events.push(FocusEvent::BreakDone { kind: *kind });
                    if self.config.auto_start_next {
                        self.begin_focus(end, &mut out);
                    } else {
                        self.go(end, Phase::Idle, &mut out);
                    }
                }
                _ => return out,
            }
        }
    }

    fn pause_with(&mut self, now: Moment, reason: PauseReason) -> Outcome {
        let mut out = Outcome::default();
        if let Phase::Focus { session, started_at, length, task } = &self.phase {
            let remaining = (*started_at + *length).since(now);
            let next = Phase::Paused { session: *session, length: *length, remaining, task: task.clone(), reason };
            self.go(now, next, &mut out);
        }
        out
    }

    fn begin_focus(&mut self, at: Moment, out: &mut Outcome) {
        if self.done_in_cycle >= self.config.sessions_per_long_break {
            self.done_in_cycle = 0;
        }
        let session = Session { n: self.done_in_cycle + 1, of: self.config.sessions_per_long_break };
        let next = Phase::Focus { session, started_at: at, length: self.last_length, task: self.last_task.clone() };
        self.go(at, next, out);
    }

    fn begin_break(&mut self, at: Moment, out: &mut Outcome) {
        self.done_in_cycle += 1;
        let (kind, length) = if self.done_in_cycle >= self.config.sessions_per_long_break {
            (BreakKind::Long, self.config.long_break)
        } else {
            (BreakKind::Short, self.config.short_break)
        };
        self.go(at, Phase::Break { kind, started_at: at, length }, out);
    }

    fn record(&mut self, at: Moment, minutes: u32, task: Option<TaskId>, out: &mut Outcome) {
        let rec = SessionRecord { date: at.date(), ended_at: at, minutes, task };
        self.log.push(rec.clone());
        out.events.push(FocusEvent::SessionLogged(rec));
    }

    fn go(&mut self, at: Moment, next: Phase, out: &mut Outcome) {
        out.transitions.push(Transition { at, from: self.phase.kind(), to: next.kind() });
        self.phase = next;
    }
}
