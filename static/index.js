const container = document.querySelector("#mount_container")
const template = document.querySelector("#mount_display")

async function get_mount_info() {
    const mount_info = await fetch("/mount_info");
    const json = await mount_info.json();
    return json
}

async function create_mount_info() {
    const mounts = await get_mount_info()
    for (const mount of mounts) {
        const display = template.content.cloneNode(true);
        display.querySelector(".rename").id = btoa(mount.name)
        container.appendChild(display)
    }
    update_mount_info(mounts)
}

function update_mount_info(mounts) {
    const mount_names = []
    for (const mount of mounts) {
        console.log(mount)
        const display = document.querySelector("#" + btoa(mount.name))
        display.hidden = false
        display.querySelector(".name").textContent = mount.name

        const subs = display.querySelector(".subs")
        subs.textContent = "Current listeners: " + mount.subscribers

        const song_name = display.querySelector(".song_name")
        if (mount.song) {
            song_name.textContent = "Now playing: " + mount.song
        }

        const on_air = display.querySelector(".on_air")
        const url_vid = display.querySelector(".stream_url")
        const url_for_copy = display.querySelector(".stream_url_copy")
        const title_p = document.querySelector(".stream_url_copy_p")


        if (mount.on_air) {
            const url = window.location.protocol + "//" + mount.stream_url
            on_air.textContent = "Yes"
            if (url_vid.src != url) {
                url_vid.src = url
                url_for_copy.value = url
            }
            url_vid.hidden = false
            url_for_copy.hidden = false
            title_p.hidden = false
            subs.hidden = false
            song_name.hidden = false
        } else {
            on_air.textContent = "No"
            url_vid.hidden = true
            url_for_copy.hidden = true
            title_p.hidden = true
            subs.hidden = true
            song_name.hidden = true
        }
        mount_names.push(btoa(mount.name))
    }

    for (const container of document.querySelectorAll(".mount_display")) {
        if (!mount_names.includes(container.id)) {
            container.remove()
        }
    }

}

create_mount_info()

setInterval(() => {
    get_mount_info().then((mounts) => update_mount_info(mounts))

}, 10000)