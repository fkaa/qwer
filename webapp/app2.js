HOSTNAME=window.location.hostname;
SIGNALING_ADDRESS="ws://"+HOSTNAME+":8080/transport/webrtc/test";

SignalingChannel = function(address) {
    this.ws = new WebSocket(address);
    // this.ws.binaryType = "arraybuffer";
    this.ws.onmessage = this.onWebSocketMessage.bind(this);
    this.onMessage = null;
}

SignalingChannel.prototype.onWebSocketMessage = function(event) {
    console.log("Got websocket data: " + event.data);

    if (this.onMessage != null) {
        this.onMessage(JSON.parse(event.data));
    }
}

SignalingChannel.prototype.onStart = function(callback) {
    this.ws.onopen = callback;
}

SignalingChannel.prototype.send = function(candidate) {
    console.log("Send: " + JSON.stringify(candidate));
    this.ws.send(JSON.stringify(candidate));
}

WebRtcVideoStream = function(video_element, signaling_address) {
    this.video = video_element;
    this.signalingChannel = new SignalingChannel(signaling_address);

    const configuration = {'iceServers': [{'urls': 'stun:stun.l.google.com:19302'}]}
    this.peerConnection = new RTCPeerConnection(configuration);

    this.peerConnection.onicecandidate = this.OnIceCandidate.bind(this);
    this.peerConnection.ontrack = this.OnTrack.bind(this);
    this.peerConnection.onsignalingstatechange = this.OnSignalingStateChange.bind(this);
    this.signalingChannel.onMessage = this.OnSignalMessage.bind(this);
    this.signalingChannel.onStart(function() {
        console.info("Starting signaling process!");
    });
}

WebRtcVideoStream.prototype.OnSignalingStateChange = function(event) {
    console.info("Signaling state: " + this.peerConnection.signalingState);
}

WebRtcVideoStream.prototype.OnIceCandidate = function(event) {
    let c = event.candidate;
    if (c) {
        this.signalingChannel.send(
            {
                'new-ice-candidate': {
                    'candidate': c.candidate,
                    'sdp_mid': c.sdpMid,
                    'sdp_mline_index': c.sdpMLineIndex,
                    'username_fragment': c.usernameFragment,
                }
            });
    }
}

WebRtcVideoStream.prototype.OnTrack = function(event) {
    console.dir(event.streams);
    this.video.srcObject = event.streams[0];
}

WebRtcVideoStream.prototype.OnSignalMessage = async function(message) {
    if (message) {
        if (message.offer != null) {
            const remoteDesc = new RTCSessionDescription(message.offer);
            await this.peerConnection.setRemoteDescription(remoteDesc);

            const answer = await this.peerConnection.createAnswer();
            await this.peerConnection.setLocalDescription(answer);
            this.signalingChannel.send({'answer': answer});
        } if (message.answer != null) {
            const remoteDesc = new RTCSessionDescription(message.answer);
            await this.peerConnection.setRemoteDescription(remoteDesc);
        } else if (message["new-ice-candidate"] != null) {
            const c = message["new-ice-candidate"];
            const candidate = new RTCIceCandidate({
                'candidate': c.candidate,
                'sdpMid': c.sdp_mid,
                'sdpMLineIndex': c.sdp_mline_index,
                'usernameFragment': c.username_fragment
            });
            await this.peerConnection.addIceCandidate(candidate);
        }
    }
}

// var video = document.getElementById("video");
// var stream = new WebRtcVideoStream(video, SIGNALING_ADDRESS);
