//! A GoldSrc connection for one player. It does not play the game. It only
//! completes the challenge and connect packets so the server runs
//! `ClientConnect`, which is the hook the example plugin logs.

use std::{
    io,
    net::{SocketAddr, UdpSocket},
    time::Duration,
};

const HEADER: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
const PLAYER_NAME: &str = "RustFake";

/// The second token of `A00000000 <challenge> ...` is the number `connect` must
/// echo.
pub fn challenge_number(packet: &[u8]) -> Option<u32> {
    let body = packet.strip_prefix(&HEADER)?;
    let text = std::str::from_utf8(body).ok()?.trim();
    let mut tokens = text.split_whitespace();
    let head = tokens.next()?;
    if !head.starts_with('A') {
        return None;
    }
    tokens.next()?.parse().ok()
}

/// Talks to `server` until it accepts `RustFake` or explains the rejection.
pub fn connect(server: SocketAddr) -> Result<(), String> {
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|error| error.to_string())?;
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;

    // "steam" selects auth protocol 3. Sven Co-op has no HLTV CD-key bypass,
    // so protocol 2 always rejects a non-Steam process. With sv_lan 1 the
    // engine still accepts protocol 3 when the ticket does not validate,
    // provided the ticket is present and protinfo raw is "steam".
    send(&socket, server, b"getchallenge steam\n")?;
    let challenge_packet = recv(&socket)?;
    let challenge = challenge_number(&challenge_packet).ok_or_else(|| {
        format!(
            "the server did not return a challenge: {}",
            preview(&challenge_packet)
        )
    })?;

    // Protocol 3 still requires a 32-character cdkey field. Sven rejects a
    // shorter one with "Invalid hashed CD key" before it looks at the ticket.
    let protinfo = r"\prot\3\unique\-1\raw\steam\cdkey\00000000000000000000000000000000";
    let userinfo = format!(r"\name\{PLAYER_NAME}\model\gordon\topcolor\0\bottomcolor\0\rate\9999");
    let mut connect =
        format!("connect 48 {challenge} \"{protinfo}\" \"{userinfo}\"\n").into_bytes();
    connect.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
    send(&socket, server, &connect)?;
    let reply = recv(&socket)?;
    if accepted(&reply) {
        // `new` is what makes the server call ClientConnect. It is a netchan
        // string command (opcode 3), not connectionless text. Sven reads the
        // first body byte as the opcode, so a bare "new" is rejected as
        // command character 110.
        send(&socket, server, &spawn_new_packet())?;
        return Ok(());
    }
    Err(format!("the server rejected RustFake: {}", preview(&reply)))
}

pub const NAME: &str = PLAYER_NAME;

fn accepted(packet: &[u8]) -> bool {
    packet.get(4).copied() == Some(b'B')
}

/// First client packet after `B`: sequence 1, ack 0, then `clc_stringcmd`
/// "new". Sven's dedicated server does not consume a client qport on this
/// packet.
fn spawn_new_packet() -> Vec<u8> {
    const CLC_STRINGCMD: u8 = 3;
    let mut packet = Vec::with_capacity(14);
    packet.extend(1u32.to_le_bytes());
    packet.extend(0u32.to_le_bytes());
    packet.push(CLC_STRINGCMD);
    packet.extend(b"new\0");
    packet
}

fn send(socket: &UdpSocket, server: SocketAddr, body: &[u8]) -> Result<(), String> {
    let mut packet = Vec::with_capacity(HEADER.len() + body.len());
    // The sequenced `new` packet is not connectionless.
    if body
        .first()
        .is_some_and(|byte| *byte != b'g' && *byte != b'c')
    {
        packet.extend(body);
    } else {
        packet.extend(HEADER);
        packet.extend(body);
    }
    socket
        .send_to(&packet, server)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn recv(socket: &UdpSocket) -> Result<Vec<u8>, String> {
    let mut buffer = [0; 2048];
    match socket.recv_from(&mut buffer) {
        Ok((size, _)) => Ok(buffer[..size].to_vec()),
        Err(error)
            if error.kind() == io::ErrorKind::WouldBlock
                || error.kind() == io::ErrorKind::TimedOut =>
        {
            Err("the server did not answer the fake client".into())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn preview(packet: &[u8]) -> String {
    String::from_utf8_lossy(packet)
        .chars()
        .map(|character| {
            if character == '\u{ff}' {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_is_the_second_token() {
        let packet = b"\xff\xff\xff\xffA00000000 4242 3 0 0\n";
        assert_eq!(challenge_number(packet), Some(4242));
    }

    #[test]
    fn a_query_response_is_not_a_challenge() {
        assert_eq!(challenge_number(b"\xff\xff\xff\xffI0"), None);
    }

    #[test]
    fn spawn_packet_is_a_string_command() {
        let packet = spawn_new_packet();
        assert_eq!(&packet[..4], &1u32.to_le_bytes());
        assert_eq!(&packet[4..8], &0u32.to_le_bytes());
        assert_eq!(&packet[8..], b"\x03new\0");
    }
}
