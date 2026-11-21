-- SPDX-License-Identifier: AGPL-3.0-or-later
-- R8 : l'entree de navigation par defaut « Observabilite » pointait vers `/observability/`, page de l'application Python qui
-- n'existe pas ici (« Page introuvable »). Elle mene aux tableaux Grafana. Une entree deja modifiee n'est pas touchee.
UPDATE navbar_item SET url = '/grafana/' WHERE url = '/observability/';
