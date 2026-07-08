import forge.card.ColorSet;
import forge.card.CardTypeView;
import forge.deck.Deck;
import forge.game.Game;
import forge.game.GameRules;
import forge.game.GameStage;
import forge.game.GameType;
import forge.game.Match;
import forge.game.card.Card;
import forge.game.card.CardFactory;
import forge.game.keyword.KeywordInterface;
import forge.game.phase.PhaseType;
import forge.game.player.Player;
import forge.game.player.RegisteredPlayer;
import forge.game.zone.ZoneType;
import forge.gui.GuiBase;
import forge.gui.download.GuiDownloadService;
import forge.gui.interfaces.IGuiBase;
import forge.gui.interfaces.IGuiGame;
import forge.item.IPaperCard;
import forge.item.PaperCard;
import forge.localinstance.properties.ForgePreferences.FPref;
import forge.localinstance.skin.FSkinProp;
import forge.localinstance.skin.ISkinImage;
import forge.model.FModel;
import forge.sound.IAudioClip;
import forge.sound.IAudioMusic;
import forge.util.FSerializableFunction;
import forge.util.ImageFetcher;
import forge.gamesimulationtests.util.LobbyPlayerForTests;

import org.jupnp.UpnpServiceConfiguration;

import java.io.File;
import java.io.IOException;
import java.net.URISyntaxException;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Comparator;
import java.util.List;
import java.util.function.Consumer;
import java.util.stream.Collectors;

public final class CpLayersLegacySnapshot {
    private CpLayersLegacySnapshot() {
    }

    public static void main(String[] args) {
        CpLayersLegacySnapshot snapshot = new CpLayersLegacySnapshot();
        snapshot.initializeModel();
        snapshot.run(args);
    }

    private void run(String[] args) {
        List<String> sources = new ArrayList<>();
        for (String arg : args) {
            if (!arg.isBlank()) {
                sources.add(arg);
            }
        }
        if (sources.isEmpty()) {
            sources.add("Humility");
        }

        for (String sourceName : sources) {
            try {
                emitScenario(sourceName);
            } catch (RuntimeException ex) {
                System.out.println("{\"scenario\":\"" + escape(sourceName) + "\",\"status\":\"error\",\"error\":\""
                        + escape(ex.toString()) + "\"}");
            }
        }
    }

    private void initializeModel() {
        GuiBase.setInterface(new SnapshotGuiBase());
        FModel.initialize(null, preferences -> {
            preferences.setPref(FPref.LOAD_CARD_SCRIPTS_LAZILY, false);
            preferences.setPref(FPref.UI_LANGUAGE, "en-US");
            return null;
        });
    }

    private Game initAndCreateGame() {
        List<RegisteredPlayer> players = new ArrayList<>();
        players.add(new RegisteredPlayer(new Deck("opponent"))
                .setPlayer(new LobbyPlayerForTests("opponent", null)));
        players.add(new RegisteredPlayer(new Deck("controller"))
                .setPlayer(new LobbyPlayerForTests("controller", null)));

        GameRules rules = new GameRules(GameType.Constructed);
        Match match = new Match(rules, players, "CP-LAYERS snapshot");
        Game game = new Game(players, rules, match);
        Player controller = game.getPlayers().get(1);
        game.setAge(GameStage.Play);
        game.getPhaseHandler().devModeSet(PhaseType.MAIN1, controller);
        game.getPhaseHandler().onStackResolved();
        return game;
    }

    private Card addCard(String name, Player player) {
        IPaperCard paperCard = FModel.getMagicDb().getCommonCards().getCard(name);
        if (paperCard == null) {
            FModel.getMagicDb().attemptToLoadCard(name);
            paperCard = FModel.getMagicDb().getCommonCards().getCard(name);
        }
        if (paperCard == null) {
            throw new IllegalArgumentException("card not found in legacy Forge DB: " + name);
        }
        Card card = Card.fromPaperCard(paperCard, player);
        card.setGameTimestamp(player.getGame().getNextTimestamp());
        player.getZone(ZoneType.Battlefield).add(card);
        return card;
    }

    private void attachIfPossible(Card source, Card firstTarget, Card fallbackTarget) {
        if (!source.isAttachment()) {
            return;
        }
        if (hasKeywordPrefix(source, "Reconfigure")) {
            return;
        }
        if ((source.isEquipment() || source.isFortification())
                && fallbackTarget != null
                && fallbackTarget.canBeAttached(source, null)) {
            source.attachToEntity(fallbackTarget, null);
            return;
        }
        if (firstTarget != null && firstTarget.canBeAttached(source, null)) {
            source.attachToEntity(firstTarget, null);
            return;
        }
        if (fallbackTarget != null && fallbackTarget.canBeAttached(source, null)) {
            source.attachToEntity(fallbackTarget, null);
        }
    }

    private boolean hasKeywordPrefix(Card card, String prefix) {
        return card.getKeywords().stream()
                .map(KeywordInterface::getOriginal)
                .anyMatch(keyword -> keyword.startsWith(prefix));
    }

    private void emitScenario(String sourceName) {
        Game game = initAndCreateGame();
        Player controller = game.getPlayers().get(1);
        Player opponent = game.getPlayers().get(0);

        Card opponentCreature = addCard("Runeclaw Bear", opponent);
        Card controllerArtifact = addCard("Memnite", controller);
        Card source = addCard(sourceName, controller);
        attachIfPossible(source, opponentCreature, controllerArtifact);
        game.getAction().checkStaticAbilities(false);

        String cards = game.getCardsIn(ZoneType.Battlefield).stream()
                .sorted(Comparator.comparing(Card::getName))
                .map(this::cardJson)
                .collect(Collectors.joining(","));
        System.out.println("{\"scenario\":\"" + escape(sourceName) + "\",\"status\":\"ok\",\"battlefield\":["
                + cards + "]}");
    }

    private String cardJson(Card card) {
        String keywords = card.getKeywords().stream()
                .map(KeywordInterface::getOriginal)
                .sorted()
                .map(CpLayersLegacySnapshot::quote)
                .collect(Collectors.joining(","));
        return "{\"name\":\"" + escape(card.getName())
                + "\",\"controller\":\"" + escape(card.getController().getName())
                + "\",\"types\":\"" + escape(typeText(card.getType()))
                + "\",\"colors\":\"" + escape(colorText(card.getColor()))
                + "\",\"power\":" + card.getNetPower()
                + ",\"toughness\":" + card.getNetToughness()
                + ",\"keywords\":[" + keywords + "]}";
    }

    private static String typeText(CardTypeView type) {
        return type == null ? "" : type.toString();
    }

    private static String colorText(ColorSet colors) {
        return colors == null ? "" : colors.toString();
    }

    private static String quote(String value) {
        return "\"" + escape(value) + "\"";
    }

    private static String escape(String value) {
        if (value == null) {
            return "";
        }
        return value.replace("\\", "\\\\").replace("\"", "\\\"");
    }

    private static final class SnapshotGuiBase implements IGuiBase {
        @Override
        public boolean isRunningOnDesktop() {
            return true;
        }

        @Override
        public boolean isLibgdxPort() {
            return false;
        }

        @Override
        public String getCurrentVersion() {
            return "cp-layers-snapshot";
        }

        @Override
        public void invokeInEdtNow(Runnable runnable) {
            runnable.run();
        }

        @Override
        public void invokeInEdtLater(Runnable runnable) {
            runnable.run();
        }

        @Override
        public void invokeInEdtAndWait(Runnable proc) {
            proc.run();
        }

        @Override
        public void runBackgroundTask(String message, Runnable task) {
            task.run();
        }

        @Override
        public boolean isGuiThread() {
            return true;
        }

        @Override
        public String getAssetsDir() {
            return new File("../forge-gui/").getAbsolutePath() + File.separator;
        }

        @Override
        public ImageFetcher getImageFetcher() {
            return null;
        }

        @Override
        public ISkinImage getSkinIcon(FSkinProp skinProp) {
            return null;
        }

        @Override
        public ISkinImage getUnskinnedIcon(String path) {
            return null;
        }

        @Override
        public ISkinImage getCardArt(PaperCard card, boolean backFace) {
            return null;
        }

        @Override
        public ISkinImage createLayeredImage(PaperCard card, FSkinProp background, String overlayFilename, float opacity) {
            return null;
        }

        @Override
        public void clearImageCache() {
        }

        @Override
        public void refreshSkin() {
        }

        @Override
        public String encodeSymbols(String str, boolean formatReminderText) {
            return str;
        }

        @Override
        public int getAvatarCount() {
            return 0;
        }

        @Override
        public int getSleevesCount() {
            return 0;
        }

        @Override
        public float getScreenScale() {
            return 1.0f;
        }

        @Override
        public void preventSystemSleep(boolean preventSleep) {
        }

        @Override
        public void download(GuiDownloadService service, Consumer<Boolean> callback) {
            callback.accept(false);
        }

        @Override
        public void copyToClipboard(String text) {
        }

        @Override
        public void browseToUrl(String url) throws IOException, URISyntaxException {
            throw new IOException("network disabled in CP-LAYERS snapshot harness");
        }

        @Override
        public void showCardList(String title, String message, List<PaperCard> list) {
        }

        @Override
        public boolean showBoxedProduct(String title, String message, List<PaperCard> list) {
            return false;
        }

        @Override
        public void showBugReportDialog(String title, String text, boolean showExitAppBtn) {
            System.err.println(title + ": " + text);
        }

        @Override
        public void showImageDialog(ISkinImage image, String message, String title) {
        }

        @Override
        public int showOptionDialog(String message, String title, FSkinProp icon, List<String> options, int defaultOption) {
            return defaultOption;
        }

        @Override
        public String showInputDialog(String message, String title, FSkinProp icon, String initialInput,
                List<String> inputOptions, boolean isNumeric) {
            if (initialInput != null) {
                return initialInput;
            }
            return inputOptions == null || inputOptions.isEmpty() ? "" : inputOptions.get(0);
        }

        @Override
        public String showFileDialog(String title, String defaultDir) {
            return null;
        }

        @Override
        public File getSaveFile(File defaultFile) {
            return defaultFile;
        }

        @Override
        public <T> List<T> order(String title, String top, int remainingObjectsMin, int remainingObjectsMax,
                List<T> sourceChoices, List<T> destChoices) {
            return sourceChoices;
        }

        @Override
        public <T> List<T> getChoices(String message, int min, int max, Collection<T> choices,
                Collection<T> selected, FSerializableFunction<T, String> display) {
            return new ArrayList<>(choices).subList(0, Math.min(max, choices.size()));
        }

        @Override
        public PaperCard chooseCard(String title, String message, List<PaperCard> list) {
            return list.isEmpty() ? null : list.get(0);
        }

        @Override
        public boolean isSupportedAudioFormat(File file) {
            return false;
        }

        @Override
        public IAudioClip createAudioClip(String filename) {
            return null;
        }

        @Override
        public IAudioMusic createAudioMusic(String filename) {
            return null;
        }

        @Override
        public void startAltSoundSystem(String filename, boolean isSynchronized) {
        }

        @Override
        public void showSpellShop() {
        }

        @Override
        public void showBazaar() {
        }

        @Override
        public IGuiGame getNewGuiGame() {
            return null;
        }

        @Override
        public forge.gamemodes.match.HostedMatch hostMatch() {
            return null;
        }

        @Override
        public UpnpServiceConfiguration getUpnpPlatformService() {
            return null;
        }

        @Override
        public boolean hasNetGame() {
            return false;
        }
    }
}
