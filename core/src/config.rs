//! Every safety threshold and tunable constant lives here, and only here
//! (CLAUDE.md "Safety rules", AUDIT P-39).
//!
//! Two profiles:
//! * [`Config::reference`] reproduces `reference/core-python/v2v/config.py` exactly.
//!   The golden tests run with it, so the Rust core keeps proving parity with the
//!   Python reference even after the product moves to the spec values.
//! * [`Config::spec_v0`] is the product profile (SPEC v0.1 §5, §6, §7, §8, §10).
//!
//! Units are SI unless a field name says otherwise (m, s, m/s, m/s², rad).

/// How the 0–100 behaviour score is computed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScoreModel {
    /// Reference: 100 − seconds over the limit − 8·harsh brakes − 5·harsh accels
    /// − 1.5·seconds tailgating − 4·overtakes in the oncoming lane.
    ReferenceTime,
    /// SPEC §7: a fixed penalty once per episode, recovering 1.2 points per clean minute.
    SpecEpisodes,
}

/// All tunable values. Construct with [`Config::reference`] or [`Config::spec_v0`].
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    // ---------------------------------------------------------------- prediction
    /// How far ahead every track is predicted (s). Reference 5.0; SPEC §6 6.0.
    pub horizon_s: f64,
    /// Prediction step (s). SPEC §6: 100 ms.
    pub pred_step_s: f64,
    /// Vehicle outlines are inflated by this much when predicting (m).
    pub safety_margin_m: f64,
    /// Each vehicle's outline is lengthened at both ends by this factor × its own reported
    /// position accuracy, so noisy GPS widens the conflict zone along the path instead of
    /// creating false certainty (SPEC §6). Lengthwise only: widening sideways would merge
    /// adjacent lanes (3.5 m apart) until map matching arrives in Step 4.
    pub uncertainty_margin_factor: f64,
    /// Warn below this predicted time to conflict (s).
    pub ttc_warn_s: f64,
    /// A conflicting vehicle that does not yield becomes Danger below this (s).
    pub ttc_brake_s: f64,
    /// When yielding, pass at least this long after the other vehicle (post-encroachment time, s).
    pub yield_pet_s: f64,
    /// Decelerations tried for the advised braking, gentlest first (m/s², ≤ 0).
    pub brake_levels: [f64; 6],
    /// Hardest braking ever advised (m/s², < 0).
    pub max_brake: f64,
    /// Advised braking while a vehicle we yield to is temporarily lost (m/s², < 0).
    pub failsafe_brake: f64,
    /// How long a lost yield target is still respected (s).
    pub yield_timeout_s: f64,
    /// Acceleration assumed when checking whether resuming speed is safe (m/s²).
    pub resume_accel: f64,
    /// Collision prediction only considers peers this close (m).
    pub safety_range_m: f64,
    /// A peer braking at least this hard ahead raises "hard braking ahead" (m/s², > 0).
    pub hard_brake_ahead_decel: f64,
    /// "Hard braking ahead" range (m).
    pub hard_brake_ahead_range_m: f64,

    // ---------------------------------------------------------------- message checks (AUDIT S-03, S-06, S-08)
    /// Speeds above this are misbehaviour (m/s).
    pub max_plausible_speed: f64,
    /// |acceleration| above this is misbehaviour (m/s²).
    pub max_plausible_accel: f64,
    /// Collision avoidance only trusts data this fresh (s).
    pub safety_max_age_s: f64,
    /// Data older than this is dropped quietly (s). Relayed data may be this old.
    pub far_max_age_s: f64,
    /// Event messages may be this old (s). SPEC §5 rule 7.
    pub event_max_age_s: f64,
    /// A timestamp this far in the future is misbehaviour (s).
    pub max_future_s: f64,
    /// Peers not heard from for this long leave the LDM (s).
    pub ldm_ttl_s: f64,
    /// One pseudonym may not send faster than this: minimum interval (s). AUDIT S-08.
    pub min_msg_interval_s: f64,
    /// (temp_id, msg_id) pairs remembered for duplicate detection (s). SPEC §5 rule 2.
    pub duplicate_cache_s: f64,
    /// Position jumps above max(speeds)·factor + slack (m/s) are misbehaviour.
    pub jump_speed_factor: f64,
    /// See `jump_speed_factor` (m/s).
    pub jump_speed_slack: f64,
    /// SPEC §10 rule instead: a jump is misbehaviour only beyond max(speeds) × time + this (m).
    /// `None` = the reference's speed rule above. The reference rule blocked honest phones
    /// with ordinary GPS jitter in the Step 4 simulation.
    pub jump_slack_m: Option<f64>,
    /// ... or this × the sender's reported accuracy if larger (user decision, Step 4: phones
    /// at ~10 m accuracy jump more than 15 m between fixes). 0 = fixed slack.
    pub jump_slack_accuracy_factor: f64,
    /// ... with the reported accuracy counted up to this (m), so a sender cannot buy an
    /// unlimited allowance by claiming poor accuracy.
    pub jump_slack_accuracy_cap_m: f64,
    /// Tracks with worse reported position accuracy never raise Warning/Danger (m). SPEC §6.
    pub max_alert_pos_accuracy_m: f64,
    /// Low-confidence tracks may still raise a Caution "possible conflict" (user decision, Step 2).
    pub low_confidence_caution: bool,

    // ---------------------------------------------------------------- tracking and map matching (SPEC §6, Step 4)
    /// Smooth each track's GPS position with a Kalman filter driven by the reported motion.
    pub tracking: bool,
    /// Position process noise of the filter (m² per s).
    pub track_process_noise_m2_s: f64,
    /// Error of the reported speed used to predict the track (m/s).
    pub track_speed_noise: f64,
    /// A fix further than this many standard deviations from the prediction restarts the track.
    pub track_reset_sigma: f64,
    /// A new pseudonym joins an old track if within this distance of its prediction (m). SPEC §6: 3 m.
    pub track_link_max_dist_m: f64,
    /// ... and this speed difference (m/s). SPEC §6: 2 m/s.
    pub track_link_max_speed_diff: f64,
    /// ... and the old track was heard within this time (s).
    pub track_link_max_gap_s: f64,
    /// Use the road map (when the platform supplies one) for the confidence gate and lane snapping.
    pub map_matching: bool,
    /// Ignore a predicted conflict that already exists now (outlines overlapping at the
    /// first step) while the pair is not closing: stopped neighbours in a queue, made to
    /// overlap by the position-uncertainty margins. Step 4.
    pub ignore_static_overlap: bool,
    /// ... "not closing" means closing slower than this (m/s).
    pub static_overlap_closing: f64,
    /// A vehicle slower than this in my path is an obstacle I must avoid whatever its
    /// direction (a stopped car across the junction, a wreck). `None` = reference rules.
    pub obstacle_speed: Option<f64>,
    /// SPEC §8 "Not yielding" also for crossing traffic, and only when the vehicle that
    /// must give way is braking less than this (m/s², > 0). `None` = reference behaviour
    /// (never for crossing traffic, no braking check).
    pub not_yielding_max_decel: Option<f64>,
    /// SPEC §8 (user decision, Step 4): "crossing conflict" and "hard braking ahead" are
    /// Danger only when the collision prediction confirms the paths meet (within
    /// `yield_pet_s` at the crossing, or the braking vehicle is inside my stopping
    /// distance); otherwise they are a Warning with the same braking advice.
    pub danger_needs_confirmation: bool,
    /// Reaction time used for "inside my stopping distance" (s).
    pub stopping_reaction_s: f64,
    /// SPEC §6 confidence gate: no Warning/Danger from tracks matched below this.
    pub map_min_confidence: f64,
    /// Heading error at which the heading likelihood falls to 0.61 (degrees).
    pub map_heading_sigma_deg: f64,
    /// Position likelihood uses this × the reported accuracy: phones report a ~68 % radius,
    /// so a fix 1× outside the road edge is still ordinary noise (Step 4 simulation).
    pub map_position_sigma_factor: f64,
    /// Snap the heading to the road only when it is within this of the road direction (degrees).
    pub map_snap_max_heading_deg: f64,
    /// ... and the reported turn rate is below this (rad/s); the turn rate is then set to 0.
    pub map_snap_max_yaw_rate: f64,
    /// Inside a junction, a track is kept on its side only if this well aligned with a road
    /// (degrees) and not turning (Step 4).
    pub map_junction_snap_max_heading_deg: f64,
    /// Same-direction vehicles cannot be put in a lane when their combined position error
    /// exceeds this (m). Such rear-end conflicts are checked with the lateral error added
    /// and raise a Warning; they become Danger only below `ttc_brake_s`. `None` = off.
    pub lane_resolve_sigma_m: Option<f64>,
    /// SPEC §10: a sender this far outside any road is misbehaviour (m). `None` = check off.
    pub off_road_max_m: Option<f64>,
    /// ... unless it is this far from every road: outside the map's coverage (m).
    pub map_coverage_m: f64,

    // ---------------------------------------------------------------- relay (SPEC §5; full rules in Step 7)
    /// Initial TTL of heartbeats: how many times they may be forwarded.
    pub heartbeat_ttl: u8,
    /// Initial TTL of event messages.
    pub event_ttl: u8,
    /// Relay only if at least this far from the previous hop (m).
    pub relay_min_progress_m: f64,

    // ---------------------------------------------------------------- hazard ahead (stopped traffic)
    /// Look this far ahead for stopped traffic (m).
    pub hazard_range_m: f64,
    /// Slower than this counts as stopped (m/s).
    pub hazard_stopped_speed: f64,
    /// Advised speed while approaching stopped traffic (m/s).
    pub hazard_advisory_speed: f64,
    /// Remember stopped traffic through relay gaps for this long (s).
    pub hazard_memory_s: f64,
    /// Advice aims to stop this far behind the queue (m).
    pub hazard_buffer_m: f64,
    /// The hazard advice never asks for harder braking than this (m/s², > 0).
    pub comfort_decel: f64,
    /// Own vehicle stopped this long (s) away from a junction sends "stopped traffic" events.
    pub stopped_event_after_s: f64,
    /// Interval between repeated "stopped traffic" events (s).
    pub stopped_event_interval_s: f64,

    // ---------------------------------------------------------------- driver advice wording
    /// Advised deceleration up to this is "brake gently" (m/s², > 0).
    pub advice_gentle_max: f64,
    /// Up to this is "brake firmly"; above is "brake hard now" (m/s², > 0).
    pub advice_firm_max: f64,

    // ---------------------------------------------------------------- behaviour (SPEC §7)
    /// Speeding when speed > limit · ratio + offset.
    pub speeding_ratio: f64,
    /// See `speeding_ratio` (m/s).
    pub speeding_offset: f64,
    /// ... sustained for this long (s).
    pub speeding_min_s: f64,
    /// Episode ends below limit · ratio + offset (with `speeding_end_offset`).
    pub speeding_end_ratio: f64,
    /// See `speeding_end_ratio` (m/s).
    pub speeding_end_offset: f64,
    /// Harsh braking at deceleration ≥ this (m/s², > 0).
    pub harsh_brake: f64,
    /// Harsh-braking episode ends below this deceleration (m/s², > 0).
    pub harsh_brake_end: f64,
    /// Harsh acceleration at ≥ this (m/s²).
    pub harsh_accel: f64,
    /// Harsh acceleration for motorbikes (m/s²).
    pub harsh_accel_motorbike: f64,
    /// Harsh-acceleration episode ends below this (m/s²).
    pub harsh_accel_end: f64,
    /// Tailgating when the time gap is below this (s).
    pub tailgate_headway_s: f64,
    /// Tailgating episode ends above this time gap (s).
    pub tailgate_end_headway_s: f64,
    /// Tailgating only counts above this speed (m/s).
    pub tailgate_min_speed: f64,
    /// ... sustained for this long (s).
    pub tailgate_min_s: f64,
    /// Overtakes and tailgating are tracked for pairs closer than this (m).
    pub behaviour_pair_range_m: f64,
    /// Behaviour ignores data older than this (s).
    pub behaviour_max_age_s: f64,
    /// Side-by-side lateral offset that makes a pass an overtake (m).
    pub overtake_min_lateral_m: f64,
    /// The pass counts once the overtaking vehicle is this far ahead (m). SPEC §7: 10.
    pub overtake_complete_m: f64,
    /// Score model.
    pub score_model: ScoreModel,
    /// SPEC §7 penalties (points).
    pub penalty_speeding: f64,
    /// See `penalty_speeding`.
    pub penalty_harsh_brake: f64,
    /// See `penalty_speeding`.
    pub penalty_harsh_accel: f64,
    /// See `penalty_speeding`.
    pub penalty_overtake: f64,
    /// See `penalty_speeding`.
    pub penalty_overtake_risky: f64,
    /// See `penalty_speeding`.
    pub penalty_tailgating: f64,
    /// Score recovery per minute of clean driving (points).
    pub score_recovery_per_min: f64,
    /// Peer behaviour records are deleted this long after the track expires (s). AUDIT S-14.
    pub peer_record_retention_s: f64,

    // ---------------------------------------------------------------- SPEC §8 alerts
    /// Use the SPEC §8 alert rules (true) or the reference's state machine (false).
    pub spec_alerts: bool,
    /// Crossing conflict: both reach the conflict point within this (s).
    pub crossing_horizon_s: f64,
    /// ... arrival times within this (s).
    pub crossing_enter_gap_s: f64,
    /// ... leaves when the arrival gap grows above this (s).
    pub crossing_exit_gap_s: f64,
    /// ... both faster than this (m/s).
    pub crossing_min_speed: f64,
    /// Hard braking ahead ends when deceleration stays under this for `hard_brake_exit_hold_s` (m/s², > 0).
    pub hard_brake_exit_decel: f64,
    /// See `hard_brake_exit_decel` (s).
    pub hard_brake_exit_hold_s: f64,
    /// "Overtaking you": peer within this beside or behind (m).
    pub overtaking_you_range_m: f64,
    /// ... ends when the peer is this far ahead (m).
    pub overtaking_you_exit_ahead_m: f64,
    /// "Fast approach from behind": range (m).
    pub approach_behind_range_m: f64,
    /// ... closing faster than this (m/s).
    pub approach_behind_enter: f64,
    /// ... ends when closing slower than this (m/s).
    pub approach_behind_exit: f64,
    /// Hard-brake events are sent on this many consecutive messages (SPEC §3: 3 within 300 ms).
    pub event_repeats: u8,
    /// Own speeding alert above limit + this (m/s).
    pub own_speeding_enter: f64,
    /// ... ends at or below limit + this (m/s).
    pub own_speeding_exit: f64,

    // ---------------------------------------------------------------- security (SPEC §10, AUDIT S-01..S-08)
    /// Certificates, revocation and signature checking. The same in both profiles.
    pub security: Security,
}

/// Security values (SPEC §10). Used by the phone core and by the server's
/// certificate authority, so both sides agree.
#[derive(Debug, Clone, PartialEq)]
pub struct Security {
    /// A phone switches pseudonym (and certificate) this often (s). SPEC §3.
    pub pseudonym_rotate_s: f64,
    /// AUDIT S-13: the switch waits until the vehicle is stopped or at a junction (where a
    /// listener cannot follow it by continuity), but never longer than this (s).
    pub rotation_max_delay_s: f64,
    /// A message carries the full certificate at least this often (s); other
    /// messages carry only its 8-byte digest (SPEC §10, user decision Step 3).
    pub full_cert_interval_s: f64,
    /// After switching pseudonym, every message carries the full certificate for this long (s).
    pub full_cert_after_rotation_s: f64,
    /// A certificate is valid for this long (s): one week.
    pub cert_validity_s: u32,
    /// Most pseudonym certificates one device may receive per `issue_window_s`. SPEC §10, AUDIT S-04.
    pub certs_per_window: usize,
    /// Rolling window of the issuance limit (s): one week.
    pub issue_window_s: u32,
    /// Independent reporting devices needed to revoke a device (CLAUDE.md: 3).
    pub revoke_after_reporters: usize,
    /// Most misbehaviour reports the server accepts from one device per day.
    pub reports_per_device_per_day: usize,
    /// An issuance request must be signed within this many seconds of the server's clock.
    pub issue_request_max_age_s: u32,
    /// Certificates and messages are accepted up to this far before their start time (s), for clock skew.
    pub clock_skew_s: f64,
    /// Verified certificates remembered by one phone.
    pub cert_cache_max: usize,
    /// Largest revocation list a phone accepts (entries of 8 bytes).
    pub crl_max_entries: usize,
    /// Most frames a misbehaviour report may carry as evidence.
    pub report_max_evidence: usize,
    /// Check only 1 in `verify_sample_every` heartbeats of already-verified peers when more
    /// than this many peers are around. `None` = check every message (v1 decision, Step 3;
    /// revisit with CPU measurements in Step 5).
    pub verify_sample_above_peers: Option<usize>,
    /// See `verify_sample_above_peers`.
    pub verify_sample_every: u32,
}

impl Security {
    /// SPEC v0.1 §10 values.
    pub fn v0() -> Self {
        Self {
            pseudonym_rotate_s: 300.0,
            rotation_max_delay_s: 300.0,
            full_cert_interval_s: 0.3, // every 3rd message at 10 Hz (user decision, Step 4)
            full_cert_after_rotation_s: 2.0,
            cert_validity_s: 7 * 24 * 3600,
            certs_per_window: 20,
            issue_window_s: 7 * 24 * 3600,
            revoke_after_reporters: 3,
            reports_per_device_per_day: 50,
            issue_request_max_age_s: 300,
            clock_skew_s: 2.0,
            cert_cache_max: 4096,
            crl_max_entries: 262_144,
            report_max_evidence: 16,
            verify_sample_above_peers: None,
            verify_sample_every: 4,
        }
    }
}

impl Config {
    /// Exact values of `reference/core-python/v2v/config.py` and `behavior.py`.
    pub fn reference() -> Self {
        Self {
            horizon_s: 5.0,
            pred_step_s: 0.1,
            safety_margin_m: 0.5,
            uncertainty_margin_factor: 0.0,
            ttc_warn_s: 4.0,
            ttc_brake_s: 2.0,
            yield_pet_s: 1.0,
            brake_levels: [0.0, -1.5, -3.0, -4.5, -6.0, -8.0],
            max_brake: -8.0,
            failsafe_brake: -3.0,
            yield_timeout_s: 3.0,
            resume_accel: 1.5,
            safety_range_m: 400.0,
            hard_brake_ahead_decel: 4.0,
            hard_brake_ahead_range_m: 200.0,

            max_plausible_speed: 70.0,
            max_plausible_accel: 12.0,
            safety_max_age_s: 0.5,
            far_max_age_s: 2.0,
            event_max_age_s: 2.0,
            max_future_s: 0.5,
            ldm_ttl_s: 1.0,
            min_msg_interval_s: 0.05,
            duplicate_cache_s: 5.0,
            jump_speed_factor: 1.5,
            jump_speed_slack: 10.0,
            jump_slack_m: None,
            jump_slack_accuracy_factor: 0.0,
            jump_slack_accuracy_cap_m: 20.0,
            max_alert_pos_accuracy_m: f64::INFINITY,
            low_confidence_caution: false,

            tracking: false,
            track_process_noise_m2_s: 0.5,
            track_speed_noise: 0.3,
            track_reset_sigma: 5.0,
            track_link_max_dist_m: 3.0,
            track_link_max_speed_diff: 2.0,
            track_link_max_gap_s: 1.5,
            map_matching: false,
            ignore_static_overlap: false,
            static_overlap_closing: 0.5,
            obstacle_speed: None,
            danger_needs_confirmation: false,
            stopping_reaction_s: 1.0,
            not_yielding_max_decel: None,
            map_min_confidence: 0.7,
            map_heading_sigma_deg: 20.0,
            map_position_sigma_factor: 2.0,
            map_snap_max_heading_deg: 30.0,
            map_snap_max_yaw_rate: 0.1,
            map_junction_snap_max_heading_deg: 15.0,
            lane_resolve_sigma_m: None,
            off_road_max_m: None,
            map_coverage_m: 1000.0,

            heartbeat_ttl: 8,
            event_ttl: 8,
            relay_min_progress_m: 100.0,

            hazard_range_m: 3000.0,
            hazard_stopped_speed: 3.0,
            hazard_advisory_speed: 90.0 / 3.6,
            hazard_memory_s: 8.0,
            hazard_buffer_m: 30.0,
            comfort_decel: 3.0,
            stopped_event_after_s: f64::INFINITY, // the reference has no stopped-traffic event
            stopped_event_interval_s: 1.0,

            advice_gentle_max: 2.0,
            advice_firm_max: 4.5,

            speeding_ratio: 1.10,
            speeding_offset: 0.0,
            speeding_min_s: 2.0,
            speeding_end_ratio: 1.10,
            speeding_end_offset: 0.0,
            harsh_brake: 4.0,
            harsh_brake_end: 2.0,
            harsh_accel: 2.5,
            harsh_accel_motorbike: 2.5,
            harsh_accel_end: 1.25,
            tailgate_headway_s: 1.0,
            tailgate_end_headway_s: 1.0,
            tailgate_min_speed: 30.0 / 3.6,
            tailgate_min_s: 2.0,
            behaviour_pair_range_m: 150.0,
            behaviour_max_age_s: 1.0,
            overtake_min_lateral_m: 1.5,
            overtake_complete_m: 0.0,
            score_model: ScoreModel::ReferenceTime,
            penalty_speeding: 0.0,
            penalty_harsh_brake: 8.0,
            penalty_harsh_accel: 5.0,
            penalty_overtake: 0.0,
            penalty_overtake_risky: 4.0,
            penalty_tailgating: 1.5,
            score_recovery_per_min: 0.0,
            peer_record_retention_s: f64::INFINITY,

            spec_alerts: false,
            crossing_horizon_s: 7.0,
            crossing_enter_gap_s: 2.2,
            crossing_exit_gap_s: 3.0,
            crossing_min_speed: 2.0,
            hard_brake_exit_decel: 1.5,
            hard_brake_exit_hold_s: 1.0,
            overtaking_you_range_m: 25.0,
            overtaking_you_exit_ahead_m: 10.0,
            approach_behind_range_m: 80.0,
            approach_behind_enter: 5.0,
            approach_behind_exit: 2.0,
            event_repeats: 0, // the reference sends no event messages
            own_speeding_enter: 5.0 / 3.6,
            own_speeding_exit: 0.0,

            security: Security::v0(),
        }
    }

    /// Product profile: SPEC v0.1 values. Prediction, right of way and the advice
    /// algorithm are the reference's; thresholds, freshness, relay hop limits,
    /// behaviour rules and alerts follow the spec.
    pub fn spec_v0() -> Self {
        let kmh = 1.0 / 3.6;
        Self {
            // §6 prediction: 6 s ahead; position uncertainty widens the conflict zone
            horizon_s: 6.0,
            uncertainty_margin_factor: 1.0,
            // §10 misbehaviour: above 200 km/h is implausible
            max_plausible_speed: 200.0 * kmh,
            // §5 rule 7: relayed heartbeats older than 1 s, events older than 10 s are dropped
            far_max_age_s: 1.0,
            event_max_age_s: 10.0,
            // §6: tracks expire 2 s after their last update
            ldm_ttl_s: 2.0,
            // AUDIT S-08: ignore a pseudonym sending more than 12 msg/s
            min_msg_interval_s: 1.0 / 12.0,
            // §6 confidence gate (8 m), with a Caution fallback (user decision, Step 2)
            max_alert_pos_accuracy_m: 8.0,
            low_confidence_caution: true,
            // §10: position jump beyond speed × time + 15 m
            jump_slack_m: Some(15.0),
            jump_slack_accuracy_factor: 3.0,
            // §6 tracking and map matching (Step 4); §10 off-road check
            tracking: true,
            map_matching: true,
            lane_resolve_sigma_m: Some(1.5),
            ignore_static_overlap: true,
            obstacle_speed: Some(2.0),
            danger_needs_confirmation: true,
            not_yielding_max_decel: Some(1.0),
            off_road_max_m: Some(50.0),
            // §5 rule 1: heartbeats TTL 2, events TTL 5
            heartbeat_ttl: 2,
            event_ttl: 5,
            // §3 send rates: an event is sent immediately and repeated 3 times within 300 ms
            event_repeats: 3,
            // stopped-traffic events (event code 5, Step 2)
            stopped_event_after_s: 5.0,
            stopped_event_interval_s: 1.0,
            // §7 behaviour
            speeding_ratio: 1.0,
            speeding_offset: 10.0 * kmh,
            speeding_min_s: 3.0,
            speeding_end_ratio: 1.0,
            speeding_end_offset: 5.0 * kmh,
            harsh_brake: 4.5,
            harsh_brake_end: 2.0,
            harsh_accel: 3.0,
            harsh_accel_motorbike: 3.5,
            harsh_accel_end: 1.5,
            tailgate_headway_s: 0.8,
            tailgate_end_headway_s: 1.2,
            tailgate_min_s: 5.0,
            overtake_complete_m: 10.0,
            score_model: ScoreModel::SpecEpisodes,
            penalty_speeding: 6.0,
            penalty_harsh_brake: 5.0,
            penalty_harsh_accel: 3.0,
            penalty_overtake: 1.0,
            penalty_overtake_risky: 4.0,
            penalty_tailgating: 2.0,
            score_recovery_per_min: 1.2,
            // §10: peer records deleted at track expiry + 10 min
            peer_record_retention_s: 600.0,
            // §8 alerts
            spec_alerts: true,
            ..Self::reference()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_profile_matches_python_config() {
        let c = Config::reference();
        assert_eq!(c.horizon_s, 5.0);
        assert_eq!(c.brake_levels, [0.0, -1.5, -3.0, -4.5, -6.0, -8.0]);
        assert_eq!(c.max_plausible_speed, 70.0);
        assert_eq!(c.min_msg_interval_s, 0.05);
        assert_eq!(c.heartbeat_ttl, 8);
        assert!((c.hazard_advisory_speed - 25.0).abs() < 1e-9);
        assert_eq!(c.score_model, ScoreModel::ReferenceTime);
        assert!(!c.spec_alerts);
        assert!(!c.tracking && !c.map_matching && c.off_road_max_m.is_none()); // not in the reference
    }

    #[test]
    fn spec_profile_matches_spec_v0_1() {
        let c = Config::spec_v0();
        let kmh = 1.0 / 3.6;
        assert_eq!(c.horizon_s, 6.0); // §6
        assert!((c.max_plausible_speed - 200.0 * kmh).abs() < 1e-9); // §10
        assert_eq!((c.heartbeat_ttl, c.event_ttl), (2, 5)); // §5 rule 1
        assert_eq!((c.far_max_age_s, c.event_max_age_s), (1.0, 10.0)); // §5 rule 7
        assert_eq!(c.ldm_ttl_s, 2.0); // §6
        assert!((c.min_msg_interval_s - 1.0 / 12.0).abs() < 1e-12); // AUDIT S-08
        assert_eq!(c.max_alert_pos_accuracy_m, 8.0); // §6
        assert!((c.speeding_offset - 10.0 * kmh).abs() < 1e-9 && c.speeding_min_s == 3.0); // §7
        assert!((c.speeding_end_offset - 5.0 * kmh).abs() < 1e-9);
        assert_eq!((c.harsh_brake, c.harsh_brake_end), (4.5, 2.0));
        assert_eq!(
            (c.harsh_accel, c.harsh_accel_motorbike, c.harsh_accel_end),
            (3.0, 3.5, 1.5)
        );
        assert_eq!(
            (
                c.tailgate_headway_s,
                c.tailgate_min_s,
                c.tailgate_end_headway_s
            ),
            (0.8, 5.0, 1.2)
        );
        assert_eq!(c.overtake_complete_m, 10.0);
        assert_eq!(
            (
                c.penalty_speeding,
                c.penalty_harsh_brake,
                c.penalty_harsh_accel,
                c.penalty_overtake,
                c.penalty_overtake_risky,
                c.penalty_tailgating
            ),
            (6.0, 5.0, 3.0, 1.0, 4.0, 2.0)
        );
        assert_eq!(c.score_recovery_per_min, 1.2);
        assert_eq!(c.peer_record_retention_s, 600.0); // §10
        assert_eq!(
            (
                c.crossing_horizon_s,
                c.crossing_enter_gap_s,
                c.crossing_exit_gap_s,
                c.crossing_min_speed
            ),
            (7.0, 2.2, 3.0, 2.0)
        ); // §8
        assert_eq!(
            (
                c.hard_brake_ahead_decel,
                c.hard_brake_ahead_range_m,
                c.hard_brake_exit_decel,
                c.hard_brake_exit_hold_s
            ),
            (4.0, 200.0, 1.5, 1.0)
        );
        assert_eq!(
            (c.overtaking_you_range_m, c.overtaking_you_exit_ahead_m),
            (25.0, 10.0)
        );
        assert_eq!(
            (
                c.approach_behind_range_m,
                c.approach_behind_enter,
                c.approach_behind_exit
            ),
            (80.0, 5.0, 2.0)
        );
        assert!((c.own_speeding_enter - 5.0 * kmh).abs() < 1e-9 && c.own_speeding_exit == 0.0);
        assert_eq!(c.event_repeats, 3); // §3
        assert!(c.tracking && c.map_matching); // §6
        assert_eq!(c.map_min_confidence, 0.7); // §6 confidence gate
        assert_eq!(
            (c.track_link_max_dist_m, c.track_link_max_speed_diff),
            (3.0, 2.0)
        ); // §6
        assert_eq!(c.off_road_max_m, Some(50.0)); // §10
        assert_eq!(c.jump_slack_m, Some(15.0)); // §10
        assert_eq!(
            (c.jump_slack_accuracy_factor, c.jump_slack_accuracy_cap_m),
            (3.0, 20.0)
        );
        assert!(c.ignore_static_overlap);
        assert!(c.danger_needs_confirmation); // user decision, Step 4
        assert_eq!(c.obstacle_speed, Some(2.0));
        assert_eq!(c.not_yielding_max_decel, Some(1.0)); // §8 "is not slowing"
        assert!(c.spec_alerts);
    }

    #[test]
    fn security_matches_spec_v0_1() {
        for c in [Config::reference(), Config::spec_v0()] {
            let s = &c.security;
            assert_eq!(s.pseudonym_rotate_s, 300.0); // §3: every 5 min
            assert_eq!((s.certs_per_window, s.issue_window_s), (20, 604_800)); // §10: 20/week
            assert_eq!(s.cert_validity_s, 604_800);
            assert_eq!(s.revoke_after_reporters, 3); // CLAUDE.md
            assert_eq!(
                (s.full_cert_interval_s, s.full_cert_after_rotation_s),
                (0.3, 2.0) // user decision, Step 4
            );
            assert_eq!(s.verify_sample_above_peers, None); // v1: verify every message
            assert_eq!(s.verify_sample_every, 4); // §10: 1 in 4 when enabled
        }
    }

    #[test]
    fn brake_levels_are_gentlest_first_and_never_positive() {
        for c in [Config::reference(), Config::spec_v0()] {
            assert!(c.brake_levels.windows(2).all(|w| w[0] > w[1]));
            assert!(c.brake_levels.iter().all(|&a| a <= 0.0));
            assert_eq!(c.brake_levels[5], c.max_brake);
        }
    }
}
