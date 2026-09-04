from pathlib import Path


PATH = Path("crates/swarm-cli/tests/discovery_network_freshness.rs")
FIELD_NAMES = [
    "authority",
    "world",
    "membership",
    "epoch",
    "fence",
    "config_sequence",
    "config_hash",
    "canonical_head",
    "sequence",
    "name",
]


def matching_paren(text: str, open_index: int) -> int:
    round_depth = 1
    square_depth = 0
    curly_depth = 0
    in_string = False
    in_char = False
    escaped = False
    i = open_index + 1
    while i < len(text):
        ch = text[i]
        if in_string:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                in_string = False
            i += 1
            continue
        if in_char:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == "'":
                in_char = False
            i += 1
            continue
        if ch == '"':
            in_string = True
        elif ch == "'":
            in_char = True
        elif ch == "(":
            round_depth += 1
        elif ch == ")":
            round_depth -= 1
            if round_depth == 0 and square_depth == 0 and curly_depth == 0:
                return i
        elif ch == "[":
            square_depth += 1
        elif ch == "]":
            square_depth -= 1
        elif ch == "{":
            curly_depth += 1
        elif ch == "}":
            curly_depth -= 1
        i += 1
    raise SystemExit("unterminated announcement(...) call")


def split_top_level_args(payload: str) -> list[str]:
    args: list[str] = []
    start = 0
    round_depth = square_depth = curly_depth = 0
    in_string = in_char = escaped = False
    for i, ch in enumerate(payload):
        if in_string:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                in_string = False
            continue
        if in_char:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == "'":
                in_char = False
            continue
        if ch == '"':
            in_string = True
        elif ch == "'":
            in_char = True
        elif ch == "(":
            round_depth += 1
        elif ch == ")":
            round_depth -= 1
        elif ch == "[":
            square_depth += 1
        elif ch == "]":
            square_depth -= 1
        elif ch == "{":
            curly_depth += 1
        elif ch == "}":
            curly_depth -= 1
        elif ch == "," and round_depth == square_depth == curly_depth == 0:
            args.append(payload[start:i].strip())
            start = i + 1
    tail = payload[start:].strip()
    if tail:
        args.append(tail)
    return args


text = PATH.read_text()
old_signature = '''fn announcement(
    authority: &PeerIdentity,
    world: swarm_protocol::WorldId,
    membership: &MembershipRecordV1,
    epoch: u64,
    fence: u64,
    config_sequence: u64,
    config_hash: Hash32,
    canonical_head: DiscoveryCanonicalHeadV1,
    sequence: u64,
    name: &str,
) -> WorldAnnouncementV1 {
'''
new_signature = '''struct AnnouncementFixture<'a> {
    authority: &'a PeerIdentity,
    world: swarm_protocol::WorldId,
    membership: &'a MembershipRecordV1,
    epoch: u64,
    fence: u64,
    config_sequence: u64,
    config_hash: Hash32,
    canonical_head: DiscoveryCanonicalHeadV1,
    sequence: u64,
    name: &'a str,
}

fn announcement(params: AnnouncementFixture<'_>) -> WorldAnnouncementV1 {
    let AnnouncementFixture {
        authority,
        world,
        membership,
        epoch,
        fence,
        config_sequence,
        config_hash,
        canonical_head,
        sequence,
        name,
    } = params;
'''
if old_signature in text:
    text = text.replace(old_signature, new_signature, 1)
elif new_signature not in text:
    raise SystemExit("missing discovery network announcement helper signature")

needle = "announcement("
search_from = 0
replacements = 0
while True:
    start = text.find(needle, search_from)
    if start < 0:
        break
    if text[max(0, start - 3) : start] == "fn ":
        search_from = start + len(needle)
        continue
    open_index = start + len("announcement")
    end = matching_paren(text, open_index)
    payload = text[open_index + 1 : end]
    args = split_top_level_args(payload)
    if len(args) == 10:
        entries = ",\n".join(
            f"        {field}" if arg == field else f"        {field}: {arg}"
            for field, arg in zip(FIELD_NAMES, args, strict=True)
        )
        replacement = "announcement(AnnouncementFixture {\n" + entries + ",\n    })"
        text = text[:start] + replacement + text[end + 1 :]
        search_from = start + len(replacement)
        replacements += 1
    else:
        search_from = end + 1

if replacements == 0 and "announcement(AnnouncementFixture {" not in text:
    raise SystemExit("no 10-argument announcement test-helper call sites found")

PATH.write_text(text)
print(f"FINAL-028 network announcement fixture structured ({replacements} call sites rewritten)")

membership_path = Path("crates/swarm-consensus/src/membership.rs")
membership_text = membership_path.read_text()
anchor = '''pub fn validate_discovery_membership_proof_shape(
    proof: &DiscoveryMembershipProofV1,
) -> Result<(), MembershipConsensusError> {
'''
canonical_guard = '''pub fn validate_discovery_membership_proof_shape(
    proof: &DiscoveryMembershipProofV1,
) -> Result<(), MembershipConsensusError> {
    let members_are_canonical = |members: &[WorldMemberV1]| {
        members
            .windows(2)
            .all(|pair| pair[0].peer_id < pair[1].peer_id)
    };
    if !members_are_canonical(&proof.initial_membership.members)
        || !members_are_canonical(&proof.current_membership.members)
        || proof.membership_certificates.iter().any(|certificate| {
            !members_are_canonical(&certificate.proposal.previous.members)
                || !members_are_canonical(&certificate.proposal.proposed.members)
        })
        || proof.pending_membership.as_ref().is_some_and(|proposal| {
            !members_are_canonical(&proposal.previous.members)
                || !members_are_canonical(&proposal.proposed.members)
        })
    {
        return Err(MembershipConsensusError::MalformedHistory);
    }
'''
if canonical_guard not in membership_text:
    if anchor not in membership_text:
        raise SystemExit("missing discovery membership proof validator anchor")
    membership_text = membership_text.replace(anchor, canonical_guard, 1)
membership_path.write_text(membership_text)
print("FINAL-028 discovery membership proof now rejects noncanonical member ordering")
