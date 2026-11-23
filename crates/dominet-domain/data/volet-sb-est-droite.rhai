// Volet-SB-est → right (close/stop): the shutter switch's right button.
// If the shutter is moving, we stop it; otherwise we close it.
// The relay has neither a `moving` key nor a `STOP` state: it publishes about one message per second while it moves, then goes quiet.
// "Moving" = its last message is less than 2 seconds old (`age_secs`).
let volet = get_device("Relais-SB-est");
if volet == () {
    throw "appareil Relais-SB-est introuvable";
}
let commande = volet.topic + "/set";
if volet.age_secs != () && volet.age_secs < 2.0 {
    publish(commande, #{ state: "STOP" });
} else {
    publish(commande, #{ state: "CLOSE" });
}
