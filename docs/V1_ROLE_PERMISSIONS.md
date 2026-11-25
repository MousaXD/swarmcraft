# V1 device roles and permission boundaries

The Worlds screen and `swarmcraft world status <world>` show a read-only
projection of locally accepted signed membership. They do not confer a live
route, quorum, hosting permit, or administration authority.

| Fact | V1 meaning |
| --- | --- |
| Member | Active, non-banned peer in the accepted signed roster |
| Player | Member; actual access still requires a verified live game route |
| Replica | Member; complete data, checkpoint verification, and background seeding are separate |
| Voter | Member, including members with hosting eligibility disabled; existing durable old/new quorum rules apply |
| Host candidate | Member whose signed roster entry grants authority eligibility |
| Local hosting consent | Machine-local opt-in and heap preference; zero or absent means no hosting consent |
| Administration | Locally accepted authority identity; actual authority operations still require backend proof |

V1 does not support independent viewer, player-only membership, or administrator
grants. Public discovery does not imply membership. The existing join option
requests signed host eligibility only, and does not opt in to hosting.
Do not change the V1 signed membership schema or relax the existing authority
gates to simulate these roles. Independent permission grants require a
separately reviewed canonical policy and migration protocol.

The Rust projection verifies genesis-anchored membership certificates, the
locally accepted epoch signature and generation, and matching descriptor,
then rechecks local accepted state. A pending join or incomplete historical
publication is displayed as synchronizing; an unverified record is displayed
as not verified. Pending membership transitions are explicitly indicated.
This is a local snapshot of role facts, not a network freshness assertion;
refresh Worlds to reread the local records. The role inspection and reading
existing consent do not create an identity when the private key is missing.
Neither desktop frontend derives new grants from the displayed role facts.
