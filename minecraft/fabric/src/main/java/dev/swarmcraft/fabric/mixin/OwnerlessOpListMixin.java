package dev.swarmcraft.fabric.mixin;

import dev.swarmcraft.fabric.OwnerlessPermissionGuard;
import net.minecraft.server.players.NameAndId;
import net.minecraft.server.players.ServerOpList;
import net.minecraft.server.players.ServerOpListEntry;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

@Mixin(ServerOpList.class)
abstract class OwnerlessOpListMixin {
    @Inject(method = "add(Lnet/minecraft/server/players/ServerOpListEntry;)Z", at = @At("HEAD"), cancellable = true)
    private void swarmcraft$denyStoredOperator(ServerOpListEntry entry, CallbackInfoReturnable<Boolean> cir) {
        if (OwnerlessPermissionGuard.active()) {
            cir.setReturnValue(false);
        }
    }

    @Inject(method = "canBypassPlayerLimit", at = @At("HEAD"), cancellable = true)
    private void swarmcraft$denyOperatorBypass(NameAndId player, CallbackInfoReturnable<Boolean> cir) {
        if (OwnerlessPermissionGuard.active()) {
            cir.setReturnValue(false);
        }
    }
}
