require('./mystyles.scss');

MseStream = require('./app.js');

const overlay = document.getElementById("overlay");
const container = document.getElementById('video-container');
const video = document.getElementById('stream');

const minTargetBuffer = 100;
const maxTargetBuffer = 2000;

play.addEventListener("click", (e) => {
    if (playBox.classList.toggle("playing")) {
        stream.attachStream();

        setPlayStatus(true);
    } else {
        stream.removeStream();

        setPlayStatus(false);
    }
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

    setLoadingStatus(false);
    setLoadingProgress(null);
};
stream.onconnectionsuccess = function(event) {
    console.log("Connected successfully");

    setLoadingStatus(true);
    setLoadingProgress(0);

    clearInterval(pollInterval);
    pollInterval = setInterval(function() {
        var bufferedMs = stream.getBufferedVideoDuration() * 1000;
        var ratio = Math.min(Math.max((bufferedMs - minTargetBuffer) / (maxTargetBuffer - minTargetBuffer), 0), 1) * 100;

        setLoadingProgress(ratio);

    }, 1000);
};
stream.onconnectionfail = function(event) {
    console.log("Failed to connect: " + event);

    clearInterval(pollInterval);
    setPlayStatus(false);
    setLoadingStatus(false);
    setLoadingProgress(100);
};
//stream.statsContainer = container;
stream.video = video;


function setPlayStatus(playing) {
    if (playing) {
        playBox.classList.add("playing");
        play.classList.remove("is-success");
        play.classList.add("is-danger");
    } else {
        playBox.classList.remove("playing");
        play.classList.remove("is-danger");
        play.classList.add("is-success");
    }
}

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

        setPlayStatus(true);
    }
}, 500);
