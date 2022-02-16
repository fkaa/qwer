require('./mystyles.scss');

MseStream = require('./app.js');

const overlay = document.getElementById("overlay");
const container = document.getElementById('video-container');
const video = document.getElementById('stream');

const minTargetBuffer = 100;
const maxTargetBuffer = 2000;

video.addEventListener("play", (e) => {
    stream.attachStream();
});

video.addEventListener("pause", (e) => {
    stream.removeStream();
});

function mapTargetBuffer(val) {
    return minTargetBuffer + (maxTargetBuffer - minTargetBuffer) * val;
}

function toTargetBuffer(val) {
    let ratio = Math.min(Math.max((val - minTargetBuffer) / (maxTargetBuffer - minTargetBuffer), 0), 1) * 100;
    return ratio;
}

const stream = new MseStream(RTMP_MSE_STREAM);

let pollInterval;

stream.onconnectstart = function(event) {
    console.log("Starting to connect");

    overlay.classList.add("hidden");
};
stream.onconnectionsuccess = function(event) {
    console.log("Connected successfully");

    clearInterval(pollInterval);
    pollInterval = setInterval(function() {
        var bufferedMs = stream.getBufferedVideoDuration() * 1000;
        var ratio = Math.min(Math.max((bufferedMs - minTargetBuffer) / (maxTargetBuffer - minTargetBuffer), 0), 1) * 100;
    }, 1000);
};
stream.onconnectionfail = function(event) {
    console.log("Failed to connect: " + event);

    clearInterval(pollInterval);
};
//stream.statsContainer = container;
stream.video = video;

function copyLogs() {
    navigator.clipboard.writeText(stream.getDebugLogs());
}

function onAutoPlayFail() {
    overlay.classList.remove("hidden");
}

let canAutoPlay = null;
video.play().then(() => video.pause()).catch(() => {
    console.log("Cannot autoplay");
    canAutoPlay = false;
    onAutoPlayFail();
});
setTimeout(() => {
    if (canAutoPlay == null) {
        console.log("Can autoplay");
        canAutoPlay = true;
        stream.attachStream();
    }
}, 500);
