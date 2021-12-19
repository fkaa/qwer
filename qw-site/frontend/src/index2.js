VideoDecoder = require('./video-decoder.js');

let decoder = new VideoDecoder(180, 180);

async function loadStreamSnapshots() {
    let images = document.querySelectorAll(".stream-poster");

    for (let i = 0; i < images.length; i++) {
        let img = images[i];

        img.src = await decoder.decode(`${TRANSPORT_ADDRESS}/snapshot/${img.getAttribute("stream-id")}`);
    }
}

loadStreamSnapshots();
