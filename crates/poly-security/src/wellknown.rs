//! `WELL_KNOWN_SID_TYPE` (`winnt.h`, values 0 to 121): the SIDs
//! `CreateWellKnownSid` builds and `IsWellKnownSid` recognises. The table
//! is the spec sheet's (observed); the identifier authorities and RIDs are
//! `winnt.h`'s, "Well-known SIDs" on Learn.

use crate::error::*;
use crate::sid::{self, Sid};

/// Types whose SID does not depend on a domain: `(type, authority, subs)`.
const FIXED: &[(u32, u64, &[u32])] = &[
    (0, 0, &[0]),
    (1, 1, &[0]),
    (2, 2, &[0]),
    (3, 3, &[0]),
    (4, 3, &[1]),
    (5, 3, &[2]),
    (6, 3, &[3]),
    (7, 5, &[]),
    (8, 5, &[1]),
    (9, 5, &[2]),
    (10, 5, &[3]),
    (11, 5, &[4]),
    (12, 5, &[6]),
    (13, 5, &[7]),
    (14, 5, &[8]),
    (15, 5, &[9]),
    (16, 5, &[10]),
    (17, 5, &[11]),
    (18, 5, &[12]),
    (19, 5, &[13]),
    (20, 5, &[14]),
    (22, 5, &[18]),
    (23, 5, &[19]),
    (24, 5, &[20]),
    (25, 5, &[32]),
    (26, 5, &[32, 544]),
    (27, 5, &[32, 545]),
    (28, 5, &[32, 546]),
    (29, 5, &[32, 547]),
    (30, 5, &[32, 548]),
    (31, 5, &[32, 549]),
    (32, 5, &[32, 550]),
    (33, 5, &[32, 551]),
    (34, 5, &[32, 552]),
    (35, 5, &[32, 554]),
    (36, 5, &[32, 555]),
    (37, 5, &[32, 556]),
    (51, 5, &[64, 10]),
    (52, 5, &[64, 21]),
    (53, 5, &[64, 14]),
    (54, 5, &[15]),
    (55, 5, &[1000]),
    (56, 5, &[32, 557]),
    (57, 5, &[32, 558]),
    (58, 5, &[32, 559]),
    (59, 5, &[32, 560]),
    (60, 5, &[32, 561]),
    (61, 5, &[32, 562]),
    (62, 5, &[32, 568]),
    (63, 5, &[17]),
    (64, 5, &[32, 569]),
    (65, 16, &[0]),
    (66, 16, &[0x1000]),
    (67, 16, &[0x2000]),
    (68, 16, &[0x3000]),
    (69, 16, &[0x4000]),
    (70, 5, &[33]),
    (71, 3, &[4]),
    (74, 5, &[22]),
    (76, 5, &[32, 573]),
    (78, 5, &[32, 574]),
    (79, 16, &[0x2100]),
    (81, 2, &[1]),
    (82, 5, &[65, 1]),
    (84, 15, &[2, 1]),
    (85, 15, &[3, 1]),
    (86, 15, &[3, 2]),
    (87, 15, &[3, 3]),
    (88, 15, &[3, 4]),
    (89, 15, &[3, 5]),
    (90, 15, &[3, 6]),
    (91, 15, &[3, 7]),
    (92, 15, &[3, 9]),
    (93, 15, &[3, 8]),
    (94, 15, &[3, 10]),
    (95, 5, &[32, 575]),
    (96, 5, &[32, 576]),
    (97, 5, &[32, 577]),
    (98, 5, &[84, 0, 0, 0, 0, 0]),
    (99, 5, &[32, 578]),
    (101, 5, &[32, 579]),
    (102, 5, &[32, 580]),
    (103, 18, &[1]),
    (104, 18, &[2]),
    (105, 5, &[113]),
    (106, 5, &[114]),
    (108, 15, &[3, 11]),
    (109, 15, &[3, 12]),
    (111, 5, &[32, 581]),
    (112, 5, &[32, 582]),
    (115, 18, &[4]),
    (116, 18, &[5]),
    (117, 18, &[6]),
    (118, 18, &[3]),
    (119, 5, &[32, 583]),
    (120, 5, &[32, 584]),
    (121, 5, &[32, 585]),
];

/// Types that are a domain SID plus a RID: `(type, rid)`.
const DOMAIN: &[(u32, u32)] = &[
    (38, 500),
    (39, 501),
    (40, 502),
    (41, 512),
    (42, 513),
    (43, 514),
    (44, 515),
    (45, 516),
    (46, 517),
    (47, 518),
    (48, 519),
    (49, 520),
    (50, 553),
    (72, 571),
    (73, 572),
    (75, 521),
    (77, 498),
    (100, 522),
    (107, 525),
    (110, 503),
    (113, 526),
    (114, 527),
];

/// `WinLogonIdsSid`.
pub const WIN_LOGON_IDS_SID: u32 = 21;
/// One past the last type `IsWellKnownSid` knows.
pub const WELL_KNOWN_TYPES: u32 = 126;

/// `CreateWellKnownSid`: the SID of `kind`, the domain SID `domain` (a
/// valid-looking SID's bytes, not checked as a domain) for the
/// domain-relative types.
pub fn create(kind: u32, domain: Option<&Sid>) -> Result<Sid, u32> {
    if let Some(&(_, auth, subs)) = FIXED.iter().find(|e| e.0 == kind) {
        return Ok(Sid::new(auth, subs));
    }
    if let Some(&(_, rid)) = DOMAIN.iter().find(|e| e.0 == kind) {
        let d = domain.ok_or(ERROR_INVALID_PARAMETER)?;
        if d.subs.len() >= sid::SID_MAX_SUB_AUTHORITIES {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let mut subs = d.subs.clone();
        subs.push(rid);
        return Ok(Sid { authority: d.authority, subs });
    }
    Err(ERROR_INVALID_PARAMETER)
}

/// `IsWellKnownSid`.
pub fn is_well_known(s: &Sid, kind: u32) -> bool {
    if let Some(&(_, auth, subs)) = FIXED.iter().find(|e| e.0 == kind) {
        return s.authority == auth && s.subs == subs;
    }
    if let Some(&(_, rid)) = DOMAIN.iter().find(|e| e.0 == kind) {
        return s.authority == 5 && s.subs.len() == 5 && s.subs[0] == 21 && s.subs[4] == rid;
    }
    kind == WIN_LOGON_IDS_SID && s.authority == 5 && s.subs.len() == 3 && s.subs[0] == 5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_consistent() {
        for kind in 0..WELL_KNOWN_TYPES {
            if let Ok(s) = create(kind, None) {
                assert!(is_well_known(&s, kind), "{kind}");
                for other in 0..WELL_KNOWN_TYPES {
                    assert_eq!(is_well_known(&s, other), other == kind, "{kind} vs {other}");
                }
            }
        }
        assert_eq!(create(26, None).unwrap().subs, [32, 544]);
        assert_eq!(create(38, None), Err(87));
        assert_eq!(create(21, None), Err(87));
        let d = Sid::new(5, &[21, 111, 222, 333]);
        assert_eq!(create(38, Some(&d)).unwrap().subs, [21, 111, 222, 333, 500]);
        assert!(is_well_known(&create(39, Some(&d)).unwrap(), 39));
        assert!(!is_well_known(&Sid::new(5, &[21, 1, 2, 3, 1001]), 38));
        assert!(is_well_known(&Sid::new(5, &[5, 0, 12345]), 21));
        assert_eq!(create(122, Some(&d)), Err(87));
        assert_eq!(create(38, Some(&Sid::new(5, &[18]))).unwrap().subs, [18, 500]);
    }
}
