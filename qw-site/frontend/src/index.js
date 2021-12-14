require('./mystyles.scss');

MseStream = require('./app.js');

const overlay = document.getElementById("overlay");
const progress = document.getElementById("video-progress");
const targetBuffer = document.getElementById("target-buffer");
const volume = document.getElementById("volume-slider");
const play = document.getElementById("play-stop-button");
const playBox = document.getElementById("play-stop-box");
const playText = document.getElementById("play-text");
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

// buffer shit
targetBuffer.addEventListener("change", (e) => {
    console.log(targetBuffer.value);
    stream.targetBuffer = mapTargetBuffer(targetBuffer.value / 100.0) / 1000.0;
});
targetBuffer.setAttribute("value", toTargetBuffer(stream.targetBuffer));

// volume shit
volume.addEventListener("change", (e) => {
    console.log(volume.value);
    video.volume = volume.value / 100.0;
});
volume.setAttribute("value", video.volume * 100);

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

function setLoadingStatus(success) {
    if (success) {
        progress.classList.remove("is-danger");
        progress.classList.add("is-success");
    } else {
        progress.classList.remove("is-success");
        progress.classList.add("is-danger");
    }
}

function setLoadingProgress(value) {
    if (value != null) {
        progress.setAttribute("value", value.toString());
    } else {
        progress.removeAttribute("value");
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
