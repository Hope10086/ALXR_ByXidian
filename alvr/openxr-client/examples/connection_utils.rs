use alvr_common::{
    data::{ClientHandshakePacket, HandshakePacket, ServerHandshakePacket},
    prelude::*,
    sockets::{CONTROL_PORT, LOCAL_IP, MAX_HANDSHAKE_PACKET_SIZE_BYTES},
};
use std::{net::Ipv4Addr,net::IpAddr, time::Duration, net::SocketAddr};
use socket2::{Socket, Domain, Type, Protocol, SockAddr};
use tokio::{net::UdpSocket, time};
use crate::{APP_CONFIG};

const CLIENT_HANDSHAKE_RESEND_INTERVAL: Duration = Duration::from_secs(1);

pub enum ConnectionError {
    ServerMessage(ServerHandshakePacket),
    NetworkUnreachable,
}

pub async fn announce_client_loop(
    handshake_packet: ClientHandshakePacket,
) -> StrResult<ConnectionError> {
        
    println!("announce_client_loop");
    println!("is localhost? {0}", APP_CONFIG.localhost);

    let control_port = if APP_CONFIG.localhost { CONTROL_PORT+1 } else { CONTROL_PORT };
    let mut handshake_socket = trace_err!(UdpSocket::bind((LOCAL_IP, control_port)).await)?;
    trace_err!(handshake_socket.set_broadcast(true))?;

    let client_handshake_packet = trace_err!(bincode::serialize(&HandshakePacket::Client(
        handshake_packet
    )))?;

    loop {
        let broadcast_result = handshake_socket
            .send_to(
                &client_handshake_packet,
                (Ipv4Addr::BROADCAST, CONTROL_PORT),
            )
            .await;
        if broadcast_result.is_err() {
            break Ok(ConnectionError::NetworkUnreachable);
        }

        let receive_response_loop = {
            let handshake_socket = &mut handshake_socket;
            async move {
                let mut server_response_buffer = [0; MAX_HANDSHAKE_PACKET_SIZE_BYTES];
                loop {
                    // this call will receive also the broadcasted client packet that must be ignored
                    let (packet_size, _) = trace_err!(
                        handshake_socket
                            .recv_from(&mut server_response_buffer)
                            .await
                    )?;

                    if let Ok(HandshakePacket::Server(handshake_packet)) =
                        bincode::deserialize(&server_response_buffer[..packet_size])
                    {
                        warn!("received packet {:?}", &handshake_packet);
                        println!("received packet {:?}", &handshake_packet);
                        break Ok(ConnectionError::ServerMessage(handshake_packet));
                    }
                }
            }
        };

        tokio::select! {
            res = receive_response_loop => break res,
            _ = time::sleep(CLIENT_HANDSHAKE_RESEND_INTERVAL) => {
                warn!("Server not found, resending handhake packet");
                println!("Server not found, resending handhake packet");
            }
        }
    }
}
