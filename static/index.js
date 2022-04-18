const container = document.querySelector("#mount_container")
const template = document.querySelector("#mount_display")

async function get_mount_info() {
    const mount_info = await fetch("/mount_info");
    const json = await mount_info.json();
    return json
}

function get_name(name) {
    var result = 'a'
    for (var i = 0; i < name.length; i++) {
        result += name.charCodeAt(i).toString(16)
    }
    return result
}

function update_mount_info(mounts) {
    const mount_names = []
    for (const mount of mounts) {
        const id = get_name(mount.name)
        let display = document.querySelector("#" + id)

        if (!display) {
            const cloned = template.content.cloneNode(true);
            cloned.querySelector(".rename").id = id
            container.appendChild(cloned)
            display = document.querySelector("#" + id)
        }

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
        mount_names.push(id)
    }

    for (const container of document.querySelectorAll(".mount_display")) {
        if (!mount_names.includes(container.id)) {
            container.remove()
        }
    }

}

get_mount_info().then(update_mount_info)
setInterval(() => {
    get_mount_info().then(update_mount_info)

}, 10000)