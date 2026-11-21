// Volet-SB-est → droite (close/stop) : bouton droit de l'interrupteur du volet.
// Si le volet est en mouvement, on l'arrête ; sinon on le ferme.
// Le relais n'a ni clé `moving` ni état `STOP` : il publie environ un message par seconde tant qu'il bouge, puis se tait.
// « En mouvement » = son dernier message a moins de 2 secondes (`age_secs`).
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
